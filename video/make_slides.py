# -*- coding: utf-8 -*-
"""Render the demo-video slides (1920x1080 PNG) from HTML with headless Edge.

Design direction: Refero Styles "Linear — midnight precision instrument"
— void canvas #08090a, paper type,
hairline borders, one acid-lime accent used sparingly, mono for numbers.
Every number on the slides comes from scripts/measure/, the CI log or the
server's 10-minute record of the deployed instance (status/10min.md).

    python video/make_slides.py            # writes video/out/slide-N.html + .png
    python video/make_slides.py --html     # html only (no Edge)

The live-page tour (video/out/tour-N.png) frames screenshots of the published
page that video/capture_page.py saved as video/out/page-N.png; without them
the tour is skipped and make_video.py stops at the slide that needs it.
"""
import io
import os
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
HERE = Path(__file__).resolve().parent
OUT = HERE / "out"
EDGE = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
RECORD = HERE.parent / "status" / "10min.md"
REOPEN = HERE.parent / "status" / "reopen_check.txt"

CSS = """
:root {
  --void:#08090a; --carbon:#0f1011; --obsidian:#161718; --graphite:#23252a; --smoke:#383b3f;
  --ash:#62666d; --fog:#8a8f98; --mist:#d0d6e0; --bone:#e5e5e6; --paper:#ffffff;
  --lime:#e4f222; --coral:#eb5757; --teal:#02b8cc;
}
* { box-sizing:border-box; margin:0; padding:0; }
html,body { width:1920px; height:1080px; background:var(--void); color:var(--paper);
  font-family:"Inter Variable","Inter","Segoe UI Variable Text","Segoe UI",system-ui,sans-serif;
  font-feature-settings:"cv01","ss03","zero"; overflow:hidden; }
.mono { font-family:"JetBrains Mono","Cascadia Mono","Consolas",ui-monospace,monospace; letter-spacing:-0.013em; }
.slide { position:relative; width:1920px; height:1080px; padding:80px 128px 250px;  /* bottom band reserved for burned subtitles */ display:flex; flex-direction:column; }
.kicker { font-size:20px; line-height:1.33; letter-spacing:-0.012em; color:var(--fog); font-weight:400; }
.kicker b { color:var(--lime); font-weight:510; }
h1 { font-size:96px; line-height:1; letter-spacing:-0.022em; font-weight:510; margin-top:24px; }
h2 { font-size:64px; line-height:1; letter-spacing:-0.022em; font-weight:510; margin-top:20px; max-width:1500px; }
.sub { font-size:32px; line-height:1.13; letter-spacing:-0.012em; color:var(--mist); font-weight:400; margin-top:28px; max-width:1500px; }
.grow { flex:1; }
.foot { display:flex; justify-content:space-between; align-items:flex-end; font-size:20px; color:var(--ash); letter-spacing:-0.012em; }
.foot .n { color:var(--fog); }
table.tight { margin-top:28px; } table.tight td { padding:10px 40px 10px 0; }
.shot { margin-top:28px; flex:1; min-height:0; display:flex; justify-content:center; align-items:flex-start; }
.shot img { max-width:100%; max-height:100%; border:0.5px solid var(--smoke); border-radius:12px; }
.big { font-size:240px; line-height:1; letter-spacing:-0.03em; font-weight:510; margin-top:40px; }
.big small { font-size:64px; color:var(--mist); letter-spacing:-0.012em; margin-left:24px; font-weight:400; }
table { border-collapse:collapse; margin-top:40px; font-size:28px; line-height:1.4; letter-spacing:-0.012em; }
th { text-align:left; color:var(--fog); font-weight:400; padding:12px 40px 12px 0; border-bottom:0.5px solid var(--smoke); }
td { padding:16px 40px 16px 0; border-bottom:0.5px solid var(--graphite); color:var(--mist); vertical-align:top; }
td.k { color:var(--paper); }
td.num { font-family:"JetBrains Mono","Cascadia Mono","Consolas",ui-monospace,monospace; color:var(--paper); }
.note { margin-top:36px; font-size:24px; line-height:1.4; color:var(--fog); max-width:1500px; }
.note b { color:var(--mist); font-weight:510; }
pre.code { margin-top:44px; background:var(--carbon); border:0.5px solid var(--smoke); border-radius:12px; padding:36px 40px;
  font-family:"JetBrains Mono","Cascadia Mono","Consolas",ui-monospace,monospace; font-size:26px; line-height:1.55; color:var(--mist); white-space:pre; }
pre.code .c { color:var(--fog); } pre.code .k { color:var(--lime); } pre.code .s { color:var(--paper); }
.boxes { display:grid; grid-template-columns:420px 96px 560px 96px 420px; align-items:center; margin-top:56px; }
.box { background:var(--carbon); border:0.5px solid var(--smoke); border-radius:12px; padding:28px 32px; font-size:26px; line-height:1.4; color:var(--mist); }
.box .t { color:var(--paper); font-weight:510; font-size:28px; margin-bottom:8px; }
.box.core { border-color:var(--lime); box-shadow:0 0 0 1px rgba(228,242,34,.25) inset; }
.arrow { text-align:center; color:var(--ash); font-size:40px; }
.stack { display:flex; flex-direction:column; gap:24px; }
.rules { margin-top:40px; font-size:26px; line-height:1.5; color:var(--mist); display:grid; grid-template-columns:1fr 1fr; gap:12px 64px; max-width:1660px; }
.rules div b { color:var(--paper); font-weight:510; }
.rules .m { font-family:"JetBrains Mono","Cascadia Mono","Consolas",ui-monospace,monospace; color:var(--lime); font-size:22px; }
.term { margin-top:36px; margin-bottom:32px; background:#000; border:0.5px solid var(--smoke); border-radius:12px; padding:32px 40px;
  font-family:"JetBrains Mono","Cascadia Mono","Consolas",ui-monospace,monospace; font-size:25px; line-height:1.5; color:var(--mist); white-space:pre; overflow:hidden; flex:1; }
.term .stale { color:var(--coral); } .term .live { color:var(--lime); } .term .dim { color:var(--ash); }
.chart { margin-top:24px; display:block; flex-shrink:0; }
.probe { margin-top:14px; background:#000; border:0.5px solid var(--smoke); border-radius:12px; padding:14px 32px; flex-shrink:0;
  font-family:"JetBrains Mono","Cascadia Mono","Consolas",ui-monospace,monospace; font-size:18px; line-height:1.4; color:var(--mist); white-space:pre; overflow:hidden; }
.probe .stale { color:var(--coral); } .probe .live { color:var(--lime); } .probe .dim { color:var(--ash); }
.pill { display:inline-block; border:0.5px solid var(--smoke); border-radius:9999px; padding:8px 18px; font-size:22px; color:var(--mist); margin-right:12px; margin-top:20px; }
.pill.on { border-color:var(--lime); color:var(--lime); }
.cols { display:grid; grid-template-columns:1fr 1fr 1fr; gap:48px; margin-top:56px; }
.col { background:var(--carbon); border:0.5px solid var(--smoke); border-radius:12px; padding:36px 40px; }
.col .t { color:var(--paper); font-size:30px; font-weight:510; margin-bottom:16px; letter-spacing:-0.012em; }
.col p { color:var(--mist); font-size:24px; line-height:1.45; }
"""

FOOT = ("AfterHours — Arbitrum Open House Singapore 2026", "{n} / {total}")


def slide(n, total, body):
    return f"""<!doctype html><html><head><meta charset="utf-8"><title>AfterHours slide {n}</title>
<style>{CSS}</style></head><body><div class="slide">{body}
</div></body></html>"""  # no footer line: it sat right above the burned-in subtitles


def demo_terminal(txt):
    out = []
    for line in txt.splitlines():
        esc = line.replace("&", "&amp;").replace("<", "&lt;")
        if "Chainlink" in esc and "h ago" in esc:
            esc = f'<span class="stale">{esc}</span>'
        elif "AfterHours:" in esc:
            esc = f'<span class="live">{esc}</span>'
        elif esc.startswith("["):
            esc = f'<span class="dim">{esc}</span>'
        out.append(esc)
    return "\n".join(out)


def load_record():
    """Rows of status/10min.md: the server's 10-minute reads of the deployed instance."""
    rows = []
    for line in io.open(RECORD, encoding="utf-8"):
        if not line.startswith("| 20"):
            continue
        f = [c.strip() for c in line.strip().strip("|").split("|")] + [""] * 8
        num = lambda v: float(v) if v else None  # noqa: E731
        rows.append({"t": datetime.strptime(f[0], "%Y-%m-%d %H:%M").replace(tzinfo=timezone.utc),
                     "session": f[2], "answer": num(f[3]), "feed": num(f[4])})
    return rows


def weekend(rows):
    """The record's first closure, as scripts/measure/reopen_check.py defines it
    (every row before the first LIVE_FEED row), and the figures it prints for it.
    Each figure is looked up in status/reopen_check.txt, so the slide cannot
    drift from the script's output."""
    k = next(i for i, r in enumerate(rows) if r["session"] == "LIVE_FEED")
    closure, nxt = rows[:k], rows[k]
    twap = [r for r in closure if r["session"].startswith("ONCHAIN_TWAP")]
    pct = lambda a, b: (a - b) / b * 100  # noqa: E731
    fig = {"friday": [r for r in closure if r["feed"] is not None][-1]["feed"],
           "last": twap[-1]["answer"], "monday": nxt["feed"], "rows": len(closure),
           "lo": min(r["answer"] for r in twap), "hi": max(r["answer"] for r in twap),
           "failed": sum(r["session"].startswith(("probe failed", "read failed")) for r in closure),
           "clamped": sum("clamped" in r["session"] for r in closure),
           "refused": sum(r["session"].startswith(("NO_DATA", "PAUSED")) for r in closure)}
    fig["e_fri"] = pct(fig["friday"], fig["monday"])
    fig["e_ah"] = pct(fig["last"], fig["monday"])
    fig["lo_pct"] = pct(fig["lo"], fig["friday"])
    fig["hi_pct"] = pct(fig["hi"], fig["friday"])
    said = io.open(REOPEN, encoding="utf-8").read()
    for key, fmt in (("friday", "$%.4f"), ("last", "$%.4f"), ("monday", "$%.4f"), ("lo", "$%.4f"),
                     ("hi", "$%.4f"), ("e_fri", "%+.3f%%"), ("e_ah", "%+.3f%%"),
                     ("lo_pct", "%+.3f%%"), ("hi_pct", "%+.3f%%")):
        assert fmt % fig[key] in said, "%s %s is not in %s" % (key, fmt % fig[key], REOPEN.name)
    assert "Closure 1: Sat 09-26 23:00 to Mon 09-28 00:00 UTC, %d rows" % fig["rows"] in said
    for gap in ("last print -0.583%, AfterHours -0.534%", "last print +0.515%, AfterHours +0.209%"):
        assert gap in said, gap
    return closure, nxt, fig


def record_svg(rows, closure, nxt, fig):
    """The weekend as recorded: Chainlink's last print (coral) against AfterHours'
    answers (lime), to the feed's first print on Monday (white dot). Direct labels
    in the right margin carry the numbers; label text stays in text colours."""
    W, H, L, R, T, B = 1664, 290, 104, 440, 44, 40
    t0, t1 = closure[0]["t"], nxt["t"] + timedelta(minutes=30)
    y0, y1 = 339.3, 341.7
    span = (t1 - t0).total_seconds()
    X = lambda t: L + (t - t0).total_seconds() / span * (W - L - R)  # noqa: E731
    Y = lambda v: T + (y1 - v) / (y1 - y0) * (H - T - B)  # noqa: E731
    shown = [r for r in rows if t0 <= r["t"] <= t1]
    g = []
    for v in (339.5, 340.0, 340.5, 341.0, 341.5):
        g.append(f'<line x1="{L}" x2="{W - R}" y1="{Y(v):.1f}" y2="{Y(v):.1f}" stroke="#23252a" stroke-width="1"/>'
                 f'<text x="{L - 16}" y="{Y(v) + 6:.1f}" text-anchor="end" class="ax">${v:.2f}</text>')
    for t, lab, anchor in ((t0, "Sat 23:00 UTC", "start"),
                           (datetime(2026, 9, 27, 6, tzinfo=timezone.utc), "Sun 06:00", "middle"),
                           (datetime(2026, 9, 27, 12, tzinfo=timezone.utc), "Sun 12:00", "middle"),
                           (datetime(2026, 9, 28, tzinfo=timezone.utc), "Mon 00:00", "end")):
        g.append(f'<text x="{X(t):.1f}" y="{H - 6}" text-anchor="{anchor}" class="ax">{lab}</text>')
    # Chainlink: the print the feed holds, as a step line
    pts = [r for r in shown if r["feed"] is not None]
    d = f"M{X(pts[0]['t']):.1f},{Y(pts[0]['feed']):.1f}"
    for r in pts[1:]:
        d += f" H{X(r['t']):.1f} V{Y(r['feed']):.1f}"
    d += f" H{X(t1):.1f}"
    g.append(f'<path d="{d}" fill="none" stroke="#eb5757" stroke-width="2.5"/>')
    # AfterHours in ONCHAIN_TWAP; a row the recorder failed to read breaks the line
    d, pen = "", False
    for r in closure:
        if r["session"].startswith("ONCHAIN_TWAP"):
            d += ("L" if pen else "M") + f"{X(r['t']):.1f},{Y(r['answer']):.1f} "
            pen = True
        else:
            pen = False
    g.append(f'<path d="{d.strip()}" fill="none" stroke="#e4f222" stroke-width="2.5" stroke-linejoin="round"/>')
    # the feed's first print after the weekend, as the record first saw it
    xm = X(nxt["t"])
    g.append(f'<line x1="{xm:.1f}" x2="{xm:.1f}" y1="{T - 8}" y2="{H - B}" stroke="#62666d" stroke-width="1" stroke-dasharray="4 5"/>')
    last_t = closure[-1]["t"]
    for x, y, c in ((X(last_t), Y(fig["friday"]), "#eb5757"), (X(last_t), Y(fig["last"]), "#e4f222"),
                    (xm, Y(fig["monday"]), "#ffffff")):
        g.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="7" fill="{c}" stroke="#08090a" stroke-width="2"/>')
    # legend, top left
    g.append(f'<rect x="{L}" y="4" width="28" height="4" rx="2" fill="#eb5757"/>'
             f'<text x="{L + 38}" y="13" class="lg">Chainlink AAPL/USD, last price</text>'
             f'<rect x="{L + 420}" y="4" width="28" height="4" rx="2" fill="#e4f222"/>'
             f'<text x="{L + 458}" y="13" class="lg">AfterHours answer, from the pool</text>')
    # direct labels, right margin
    lx = W - R + 40
    # the last two labels sit about 55 px apart, so each is nudged away from the other
    for v, nudge, name, val, c in ((fig["friday"], 0, "Chainlink, Friday's price", "$%.4f  %+.3f%%" % (fig["friday"], fig["e_fri"]), "#eb5757"),
                                   (fig["monday"], -8, "Chainlink, back on Monday", "$%.4f" % fig["monday"], "#ffffff"),
                                   (fig["last"], 10, "AfterHours, last weekend answer", "$%.4f  %+.3f%%" % (fig["last"], fig["e_ah"]), "#e4f222")):
        y = Y(v) + nudge
        g.append(f'<rect x="{lx - 22}" y="{y - 22:.1f}" width="6" height="46" rx="3" fill="{c}"/>'
                 f'<text x="{lx}" y="{y - 4:.1f}" class="ln">{name}</text>'
                 f'<text x="{lx}" y="{y + 24:.1f}" class="lv">{val}</text>')
    g.append(f'<text x="{lx - 22}" y="13" class="lg">vs Chainlink\'s new price on Monday</text>')
    style = ('<style>.ax{font:18px "JetBrains Mono","Cascadia Mono",Consolas,monospace;fill:#8a8f98}'
             '.lg{font:19px "Inter","Segoe UI",sans-serif;fill:#d0d6e0}'
             '.ln{font:19px "Inter","Segoe UI",sans-serif;fill:#8a8f98}'
             '.lv{font:500 25px "JetBrains Mono","Cascadia Mono",Consolas,monospace;fill:#ffffff}</style>')
    return f'<svg class="chart" width="{W}" height="{H}" viewBox="0 0 {W} {H}">{style}{"".join(g)}</svg>'


def demo_slide():
    """Slide 6 shows only real reads: the weekend from the server's 10-minute
    record of the deployed instance, and one scripts/probe.py read saved to
    video/probe.txt. Without probe.txt the strip says the read is pending."""
    rows = load_record()
    closure, nxt, fig = weekend(rows)
    probe = HERE / "probe.txt"
    if probe.exists():
        txt = io.open(probe, encoding="utf-8").read().rstrip().splitlines()
        k = next(i for i, line in enumerate(txt) if line.startswith("["))
        strip = demo_terminal("\n".join(txt[k:]))
    else:
        strip = ('<span class="dim">read pending: python scripts/probe.py --oracle '
                 '0x88b628472e595725178cc3e5e2ec70ada67f80f0 &gt; video/probe.txt</span>')
    return f"""<div class="kicker">Robinhood Chain mainnet · AfterHours 0x88b6…80f0, the oracle of a Morpho AAPL/USDG market · read every 10 minutes from a server · <b>status/10min.md</b> · <b>reopen_check.py</b></div>
<h2>Live on mainnet, pricing a Morpho market.<br>A real weekend, up to Monday's reopening.</h2>
{record_svg(rows, closure, nxt, fig)}
<div class="note" style="margin-top:12px;font-size:22px">{fig['rows']} weekend rows · answers ${fig['lo']:.4f} to ${fig['hi']:.4f} ({fig['lo_pct']:+.3f}% to {fig['hi_pct']:+.3f}% of Friday's price) · {fig['clamped']} clamped · {fig['refused']} refused · {fig['failed']} read lost on the recorder's side.<br><b>Two weekday gaps after it, vs Chainlink's next price: its last price -0.583% and +0.515%, AfterHours -0.534% and +0.209%.</b></div>
<div class="kicker" style="margin-top:18px">The same instance today · <b>python scripts/probe.py --oracle 0x88b6…80f0</b></div>
<div class="probe">{strip}</div>
<div class="grow"></div>"""


def slides():
    S = []
    S.append("""<div class="kicker">Robinhood Chain · Arbitrum Stylus · <b>Rust</b></div>
<h1>AfterHours</h1>
<div class="sub">A 24/7 price oracle for tokenized stocks.<br>Chainlink while the market is open; the on-chain pool, held near Chainlink's last price, while it is closed.</div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">Measured on mainnet · AAPL/USD feed rounds · <b>scripts/measure/feed_cadence.py</b> · <b>weekend_swaps.py</b></div>
<h2>The feed stops every weekend.</h2>
<div class="big">52–57<small>hours silent, every weekend</small></div>
<table><tr><th>window</th><th>last print → first print</th><th>silence</th></tr>
<tr><td class="k">Ordinary weekend</td><td>Fri 09-11 19:51 → Mon 09-14 00:00 UTC</td><td class="num">52.2 h</td></tr>
<tr><td class="k">Ordinary weekend</td><td>Fri 09-18 15:11 → Mon 09-21 00:00 UTC</td><td class="num">56.8 h</td></tr>
<tr><td class="k">Ordinary weekend</td><td>Fri 09-25 19:49 → Mon 09-28 00:00 UTC</td><td class="num">52.2 h</td></tr>
<tr><td class="k">Labor Day weekend</td><td>Fri 09-04 19:51 → Tue 09-08 00:00 UTC</td><td class="num">76.2 h</td></tr></table>
<div class="note">The 24 h heartbeat is not honoured during the closure. That is by design: <b>us_equities_24/5</b>.</div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">Swap events of the three AAPL/USDG pools inside the silent windows · <b>scripts/measure/weekend_swaps.py</b></div>
<h2>The token keeps trading while the feed sleeps.</h2>
<div class="big" style="margin-top:24px">$11.9M<small>of one stock, four weekends</small></div>
<table class="tight"><tr><th>weekend</th><th>swaps</th><th>volume</th><th>fill vs Monday's opening print</th></tr>
<tr><td class="k">Labor Day (76 h)</td><td class="num">44,338</td><td class="num">$5.14M</td><td>median 0.35% · p90 0.56% · max 1.67%</td></tr>
<tr><td class="k">09-11 → 09-14 (52 h)</td><td class="num">26,331</td><td class="num">$4.17M</td><td>median 0.45% · p90 0.59% · max 2.26%</td></tr>
<tr><td class="k">09-18 → 09-21 (57 h)</td><td class="num">5,324</td><td class="num">$2.03M</td><td>median 0.15% · p90 0.41% · max 2.63%</td></tr>
<tr><td class="k">09-25 → 09-28 (52 h)</td><td class="num">1,770</td><td class="num">$0.58M</td><td>median 0.08% · p90 0.25% · max 0.56%</td></tr></table>
<div class="note">Flow fell through September; every weekend the token still traded at prices that moved. <b>Lending contracts ignore that price or read it raw.</b></div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">81 funded Morpho markets on stock collateral · <b>scripts/measure/morpho_markets.py --borrows-since 2026-09-16</b> · 2026-09-30</div>
<h2>Lending against stocks lives with the stale price.</h2>
<div class="big">$732k<small>borrowed of $756k supplied</small></div>
<table><tr><th>borrowed (UTC)</th><th>amount</th><th>collateral</th><th>AAPL's Chainlink feed</th></tr>
<tr><td class="k">Sun 09-27 04:57 · one transaction</td><td class="num">$300k</td><td>NVDA · SPCX · AAPL</td><td>silent since Friday 19:49</td></tr>
<tr><td class="k">Mon 09-28 17:27 · one transaction</td><td class="num">$300k</td><td>GOOGL · AAPL · SPCX</td><td>market open</td></tr>
<tr><td class="k">the rest since 09-16</td><td class="num">$132k</td><td>mostly NVDA</td><td></td></tr></table>
<div class="note">A week earlier, $6.4k was borrowed. These markets' oracles answer <b>Friday's print</b> all weekend; outside them, PARE's pSPY lending oracle accepts a <b>five-day-old</b> one.</div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">The product · one contract per asset · <b>drop-in</b> for a Chainlink address</div>
<h2>AfterHours sits in front of the feed.</h2>
<div class="boxes" style="margin-top:36px">
  <div class="stack">
    <div class="box"><div class="t">Chainlink feed</div>24/5 · last print per token</div>
    <div class="box"><div class="t">Uniswap v3 pool</div>fixed venue · 24/7 TWAP</div>
    <div class="box"><div class="t">Stock token</div>pause flag</div>
  </div>
  <div class="arrow">→</div>
  <div class="box core"><div class="t">AfterHours</div>Stylus, Rust · immutable · no owner<br><span class="mono" style="color:var(--fog)">LIVE_FEED · ONCHAIN_TWAP · PAUSED · NO_DATA</span></div>
  <div class="arrow">→</div>
  <div class="stack">
    <div class="box"><div class="t">latestRoundData()</div>AggregatorV3 + v2 getters</div>
    <div class="box"><div class="t">price()</div>Morpho Blue IOracle, 1e36</div>
  </div>
</div>
<div class="rules">
  <div><b>Fresh feed</b> → pass it through, verbatim.</div>
  <div><b>Quiet feed</b> → pool median, <span class="m">±1% / ±10%</span> of Chainlink's last price.</div>
  <div><b>Thin pool</b> (3-window median) → refuse, never switch venue.</div>
  <div><b>Paused, or print &gt;5 days</b> → refuse.</div>
</div>
<div class="grow"></div>""")

    S.append(demo_slide())

    S.append("""<div class="kicker">Contract quality · Rust on Arbitrum Stylus · live on Robinhood Chain mainnet since 2026-09-26 · <b>github.com/bongbongcrypto/afterhours</b></div>
<h2>No owner, no upgrade, every number traceable.</h2>
<div class="cols">
  <div class="col"><div class="t">73 tests + 90 on-chain assertions</div><p>Unit and property tests with exact calldata mocks; the real wasm deployed on a local Arbitrum node (ArbOS 61), every session, both bands, a one-window spike, the venue rule and a 10:1 split priced continuously, gas per read measured.</p></div>
  <div class="col"><div class="t">Tick math vs 80-digit references</div><p>1.0001^tick in Q96 with 512-bit intermediates, checked against independently computed vectors — no magic constants.</p></div>
  <div class="col"><div class="t">11 reviews + 1 self-review</div><p>Anchor-age cap, a fixed venue an attacker cannot redirect, price and liquidity judged over three sub-windows, a narrow band only while the exchange is open, units that match Chainlink's per-token feed through dividends and splits. 14 stocks meet the bar today; 18 pass initialize.</p></div>
</div>
<div><span class="pill on">live on mainnet · 0x88b6…80f0 · oracle of a Morpho AAPL/USDG market</span><span class="pill">gas per read on mainnet: 118,426 (feed) · 214,652–247,505 (pool)</span><span class="pill">cargo stylus check ✓ 38 KB</span></div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">What it unlocks</div>
<h2>Lending that can liquidate on Saturday.<br>Automation that runs seven days.<br>One answer to what a stock is worth while the exchange is closed.</h2>
<div class="note" style="font-size:28px;margin-top:48px">Live on Robinhood Chain mainnet since 2026-09-26 as <b>the oracle of a Morpho AAPL/USDG market</b>; its first weekend was recorded every 10 minutes up to <b>Monday's reopening</b>.</div>
<div class="grow"></div>
<div class="sub">AfterHours — <span class="mono" style="color:var(--fog)">github.com/bongbongcrypto/afterhours</span></div>""")
    return S


TOUR = [
    ("The live page · <b>bongbongcrypto.github.io/afterhours</b> · read from Robinhood Chain by your browser", "page-1.png"),
    ("Chainlink's last print, the deployed instance's <b>state()</b>, and the contract's rules re-run in the page on the same block", "page-2.png"),
    ("The same rules on every stock that qualifies · <b>only AAPL is deployed</b>", "page-3.png"),
]


def tour_slides():
    """One framed screenshot per narration line of the tour slide."""
    out = []
    for kicker, shot in TOUR:
        if not (OUT / shot).exists():
            return []
        out.append("""<div class="kicker">%s</div>
<div class="shot"><img src="%s" alt=""></div>""" % (kicker, shot))
    return out


def shoot(html, png):
    if png.exists():
        png.unlink()
    # fresh profile per capture: a reused profile serves the cached old page
    profile = tempfile.mkdtemp(prefix="afterhours-edge-")
    url = html.resolve().as_uri() + "?v=%d" % int(time.time() * 1000)
    # --no-default-browser-check: without it, a current Edge exits before capturing
    cmd = [EDGE, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run", "--no-default-browser-check",
           "--user-data-dir=" + profile, "--window-size=1920,1080",
           "--screenshot=" + str(png), url]
    subprocess.run(cmd, capture_output=True, timeout=120)
    # the launcher can return before its child has written the file
    for _ in range(40):
        if png.exists() and png.stat().st_size >= 10_000:
            break
        time.sleep(0.25)
    shutil.rmtree(profile, ignore_errors=True)
    if not png.exists() or png.stat().st_size < 10_000:
        sys.exit("capture failed for %s" % png)
    return png.stat().st_size


def render(html_only):
    OUT.mkdir(parents=True, exist_ok=True)
    S = slides()
    T = tour_slides()
    for i, body in enumerate(S, 1):
        html = OUT / f"slide-{i}.html"
        io.open(html, "w", encoding="utf-8", newline="\n").write(slide(i, len(S), body))
    for i, body in enumerate(T, 1):
        html = OUT / f"tour-{i}.html"
        io.open(html, "w", encoding="utf-8", newline="\n").write(slide(i, len(T), body))
    print("html: %d slides and %d tour frames in %s" % (len(S), len(T), OUT))
    if html_only:
        return
    if not os.path.exists(EDGE):
        sys.exit("Edge not found at %s" % EDGE)
    sizes = set()
    names = [f"slide-{i}" for i in range(1, len(S) + 1)] + [f"tour-{i}" for i in range(1, len(T) + 1)]
    for name in names:
        size = shoot(OUT / (name + ".html"), OUT / (name + ".png"))
        assert size not in sizes, "two slides rendered to byte-identical PNGs (cached page?)"
        sizes.add(size)
        print("  %s.png  %d bytes" % (name, size))


if __name__ == "__main__":
    render(html_only="--html" in sys.argv)
