# -*- coding: utf-8 -*-
"""How close was AfterHours to the feed's next print? Reads the server's
10-minute record (status/10min.md, rows written by `scripts/probe.py --json`)
and, for every closure in it, compares three prices with the first print the
feed made after the closure:

  - the last Chainlink print before the closure (what a stale-feed oracle
    keeps answering; on a weekend, Friday's print),
  - AfterHours' last ONCHAIN_TWAP answer before the feed printed again,
  - the first new Chainlink print (on a weekend, Monday's).

A closure is a run of rows not in LIVE_FEED (the feed older than the
instance's liveMaxAge, six hours for AAPL); it ends at the next LIVE_FEED row.
Rows that say "probe failed" or "read failed" have no answer: they neither
start nor end a closure and are counted. The feed's print time is estimated as
the row's time minus the feed age, which the record rounds to 0.1 h, so it is
good to about three minutes (a new print is never placed before the last row
that did not see it); the "first new print" is the first print the 10-minute
record saw.

Read-only, stdlib only, no network.

    python scripts/measure/reopen_check.py [path/to/10min.md]
"""
import io
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
DEFAULT = Path(__file__).resolve().parents[2] / "status" / "10min.md"
STEP = timedelta(minutes=10)


def parse(path):
    rows = []
    for line in io.open(path, encoding="utf-8"):
        if not line.startswith("| 20"):
            continue
        f = [c.strip() for c in line.strip().strip("|").split("|")]
        f += [""] * (8 - len(f))
        t = datetime.strptime(f[0], "%Y-%m-%d %H:%M").replace(tzinfo=timezone.utc)
        num = lambda s: float(s) if s else None  # noqa: E731
        rows.append({"t": t, "asset": f[1], "session": f[2], "answer": num(f[3]),
                     "feed": num(f[4]), "age": num(f[5])})
    return rows


def failed(r):
    return r["session"].startswith(("probe failed", "read failed"))


def live(r):
    return r["session"] == "LIVE_FEED"


def day(t):
    return t.strftime("%a %m-%d %H:%M")


def printed(r, after=None):
    """The print time implied by a row: row time minus the feed age. A print
    the previous row did not see came after that row, so the estimate is
    raised to that row's time when the rounding puts it earlier."""
    t = r["t"] - timedelta(hours=r["age"])
    return max(t, after) if after is not None else t


def pct(a, b):
    return (a - b) / b * 100


def closures(rows):
    """[(rows of the closure, first LIVE_FEED row after it or None), ...]"""
    out, cur = [], None
    for r in rows:
        if failed(r):
            if cur is not None:
                cur.append(r)
            continue
        if live(r):
            if cur is not None:
                while cur and failed(cur[-1]):  # a failed read just before the print belongs to neither
                    cur.pop()
                out.append((cur, r))
                cur = None
        elif cur is None:
            cur = [r]
        else:
            cur.append(r)
    if cur is not None:
        out.append((cur, None))
    return out


def main():
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT
    rows = parse(path)
    if not rows:
        sys.exit("no rows in %s" % path)
    gaps = [(a["t"], b["t"]) for a, b in zip(rows, rows[1:]) if b["t"] - a["t"] != STEP]
    print("%s: %d rows, %s to %s UTC, asset %s"
          % (path.name, len(rows), day(rows[0]["t"]), day(rows[-1]["t"]),
             ", ".join(sorted({r["asset"] for r in rows}))))
    print("  %d rows in LIVE_FEED, %d not, %d without a read (probe or read failed); %s"
          % (sum(live(r) for r in rows), sum(not live(r) and not failed(r) for r in rows),
             sum(failed(r) for r in rows),
             "no missing 10-minute slot" if not gaps else "slots missing after: "
             + ", ".join(day(a) for a, _ in gaps)))
    summary = []
    for i, (cl, nxt) in enumerate(closures(rows), 1):
        valid = [r for r in cl if not failed(r)]
        twap = [r for r in valid if r["session"].startswith("ONCHAIN_TWAP") and r["answer"] is not None]
        stale = valid[-1]
        feeds = sorted({r["feed"] for r in valid})
        head = "Closure %d: %s to %s UTC, %d rows" % (i, day(cl[0]["t"]), day(cl[-1]["t"]), len(cl))
        start = next(k for k, r in enumerate(rows) if r is cl[0])
        if all(failed(r) for r in rows[:start]):
            head += " (the record starts inside it)"
        print("\n" + head)
        print("  last print before     $%.4f  Chainlink, printed ~%s UTC" % (stale["feed"], day(printed(stale))))
        if len(feeds) > 1:
            print("    (the Chainlink column changed inside the closure: %s)" % ", ".join("%.4f" % v for v in feeds))
        if twap:
            last = twap[-1]
            print("  AfterHours last       $%.4f  ONCHAIN_TWAP at %s UTC" % (last["answer"], day(last["t"])))
        else:
            last = None
            print("  AfterHours last       none (no ONCHAIN_TWAP row in this closure)")
        if nxt is None:
            print("  first new print       none yet: the record ends inside this closure")
        else:
            print("  first new print       $%.4f  Chainlink, printed ~%s UTC (first LIVE_FEED row %s UTC)"
                  % (nxt["feed"], day(printed(nxt, stale["t"])), day(nxt["t"])))
            e_stale = pct(stale["feed"], nxt["feed"])
            line = "  error vs the new print: last print %+.3f%%" % e_stale
            if last is not None:
                e_ah = pct(last["answer"], nxt["feed"])
                line += ", AfterHours %+.3f%%" % e_ah
                line += " (AfterHours closer)" if abs(e_ah) < abs(e_stale) else (
                    " (the last print closer)" if abs(e_ah) > abs(e_stale) else " (equal)")
                summary.append((i, day(printed(stale)), stale["feed"], last["answer"], nxt["feed"],
                                day(printed(nxt, stale["t"])), e_stale, e_ah))
            print(line)
        if twap:
            lo = min(twap, key=lambda r: r["answer"])
            hi = max(twap, key=lambda r: r["answer"])
            print("  AfterHours answers    min $%.4f (%s), max $%.4f (%s); %+.3f%% to %+.3f%% of the last print"
                  % (lo["answer"], day(lo["t"]), hi["answer"], day(hi["t"]),
                     pct(lo["answer"], stale["feed"]), pct(hi["answer"], stale["feed"])))
        clamped = [r for r in valid if "clamped" in r["session"]]
        nodata = [r for r in valid if r["session"].startswith("NO_DATA")]
        paused = [r for r in valid if r["session"].startswith("PAUSED")]
        fails = [r for r in cl if failed(r)]
        print("  clamped rows %d, NO_DATA rows %d, PAUSED rows %d; rows without a read %d%s"
              % (len(clamped), len(nodata), len(paused), len(fails),
                 (" (" + ", ".join("%s %s" % (day(r["t"]), r["session"]) for r in fails) + ")") if fails else ""))
    if summary:
        print("\nSummary (error = (price - first new print) / first new print)")
        print("  %-3s %-18s %10s %10s %10s %-18s %9s %9s"
              % ("#", "last print at", "last print", "AfterHours", "new print", "new print at", "err last", "err AH"))
        for s in summary:
            print("  %-3d %-18s %10.4f %10.4f %10.4f %-18s %+8.3f%% %+8.3f%%" % s)


if __name__ == "__main__":
    main()
