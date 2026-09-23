# -*- coding: utf-8 -*-
"""Render the demo-video slides (1920x1080 PNG) from HTML with headless Edge.

Design direction: Refero Styles "Linear — midnight precision instrument"
— void canvas #08090a, paper type,
hairline borders, one acid-lime accent used sparingly, mono for numbers.
Every number on the slides comes from scripts/measure/ or the CI log.

    python video/make_slides.py            # writes video/out/slide-N.html + .png
    python video/make_slides.py --html     # html only (no Edge)
    python video/make_slides.py --capture-page http://localhost:8745/   # + a capture of the live page for slide 6
"""
import io
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
HERE = Path(__file__).resolve().parent
OUT = HERE / "out"
EDGE = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"

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
.shot { margin-top:32px; border:0.5px solid var(--smoke); border-radius:12px; overflow:hidden; flex:1; min-height:0; }
.shot img { display:block; width:100%; height:100%; object-fit:cover; object-position:50% 100%; }
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
<div class="foot"><span>{FOOT[0]}</span><span class="n">{FOOT[1].format(n=n, total=total)}</span></div>
</div></body></html>"""


def demo_terminal(txt):
    out = []
    for line in txt.splitlines():
        esc = line.replace("&", "&amp;").replace("<", "&lt;")
        if "Chainlink" in esc and "h ago" in esc:
            esc = f'<span class="stale">{esc}</span>'
        elif "AfterHours:" in esc:
            esc = f'<span class="live">{esc}</span>'
        elif esc.startswith("[") or "placeholder" in esc:
            esc = f'<span class="dim">{esc}</span>'
        out.append(esc)
    return "\n".join(out)


def demo_slide():
    """Slide 6 shows only real reads. With a deployed instance: scripts/probe.py's
    output saved to video/probe.txt. Before that: a capture of the live page,
    which runs the contract's rules on mainnet data in the browser (made with
    --capture-page). With neither, the slide says so and shows no numbers."""
    probe = HERE / "probe.txt"
    shot = OUT / "page-capture.png"
    when = OUT / "page-capture.txt"
    if probe.exists():
        txt = io.open(probe, encoding="utf-8").read().rstrip()
        return f"""<div class="kicker">Robinhood Chain mainnet · <b>scripts/probe.py</b> against the deployed instance · the feed asleep, the oracle awake</div>
<h2>The print is hours old. AfterHours answers.</h2>
<div class="term">{demo_terminal(txt)}</div>"""
    if shot.exists() and when.exists():
        at = io.open(when, encoding="utf-8").read().strip()
        return f"""<div class="kicker">Robinhood Chain mainnet, {at} · <b>the live page</b> · the contract's rules run in the browser; not deployed yet</div>
<h2>The print is hours old. The pool answers.</h2>
<div class="shot"><img src="page-capture.png" alt="The live page at {at}"></div>"""
    return """<div class="kicker">Robinhood Chain mainnet · capture pending</div>
<h2>The live capture goes here.</h2>
<div class="note">Run <b>python video/make_slides.py --capture-page http://localhost:8745/</b> with the page served, or save scripts/probe.py output to video/probe.txt after deployment.</div>
<div class="grow"></div>"""


def capture_page(url):
    """Screenshot the live page for slide 6. The page reads Robinhood Chain from
    the browser, so Edge gets a virtual-time budget to finish its reads."""
    OUT.mkdir(parents=True, exist_ok=True)
    png = OUT / "page-capture.png"
    if png.exists():
        png.unlink()
    profile = tempfile.mkdtemp(prefix="afterhours-edge-")
    cmd = [EDGE, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run", "--no-default-browser-check",
           "--user-data-dir=" + profile, "--window-size=1600,1000", "--virtual-time-budget=30000",
           "--screenshot=" + str(png), url]
    subprocess.run(cmd, capture_output=True, timeout=180)
    for _ in range(80):
        if png.exists() and png.stat().st_size >= 10_000:
            break
        time.sleep(0.25)
    shutil.rmtree(profile, ignore_errors=True)
    if not png.exists() or png.stat().st_size < 10_000:
        sys.exit("page capture failed (%s)" % png)
    at = time.strftime("%a %d %b %Y %H:%M UTC", time.gmtime())
    io.open(OUT / "page-capture.txt", "w", encoding="utf-8").write(at + "\n")
    print("page capture %s  %d bytes  %s" % (png, png.stat().st_size, at))


def slides():
    S = []
    S.append("""<div class="kicker">Robinhood Chain · Arbitrum Stylus · <b>Rust</b></div>
<h1>AfterHours</h1>
<div class="sub">A 24/7 price for tokenized stocks.<br>Chainlink while the market is open — the on-chain pool, bounded, while it is closed.</div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">Measured on mainnet · AAPL/USD feed rounds · <b>scripts/measure/feed_cadence.py</b></div>
<h2>The feed stops every weekend.</h2>
<div class="big">52–57<small>hours silent, every weekend</small></div>
<table><tr><th>window</th><th>last print → first print</th><th>silence</th></tr>
<tr><td class="k">Ordinary weekend</td><td>Fri 09-11 19:51 → Mon 09-14 00:00 UTC</td><td class="num">52.2 h</td></tr>
<tr><td class="k">Ordinary weekend</td><td>Fri 09-18 15:11 → Mon 09-21 00:00 UTC</td><td class="num">56.8 h</td></tr>
<tr><td class="k">Labor Day weekend</td><td>Fri 09-04 19:51 → Tue 09-08 00:00 UTC</td><td class="num">76.2 h</td></tr></table>
<div class="note">The 24 h heartbeat is not honoured during the closure. That is by design: <b>us_equities_24/5</b>.</div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">Swap events of the three AAPL/USDG pools inside the silent windows · <b>scripts/measure/weekend_swaps.py</b></div>
<h2>The token keeps trading while the feed sleeps.</h2>
<div class="big">$9.3M<small>of one stock, two weekends</small></div>
<table><tr><th>weekend</th><th>swaps</th><th>volume</th><th>fill vs Monday's opening print</th></tr>
<tr><td class="k">Labor Day (76 h)</td><td class="num">44,338</td><td class="num">$5.14M</td><td>median 0.35% · p90 0.56% · max 1.67%</td></tr>
<tr><td class="k">09-11 → 09-14 (52 h)</td><td class="num">26,331</td><td class="num">$4.17M</td><td>median 0.45% · p90 0.59% · max 2.26%</td></tr></table>
<div class="note">The pool price moved through the weekend (+0.56% → −0.23% against the open). <b>Lending contracts ignore it or read it raw.</b></div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">83 funded Morpho markets on stock collateral · <b>scripts/measure/morpho_markets.py</b> · 2026-09-23</div>
<h2>Lending against stocks lives with the stale price.</h2>
<div class="big">$6.4k<small>borrowed of $0.88M supplied</small></div>
<table><tr><th>oracle behind the market</th><th>markets</th><th>supplied</th><th>on a weekend it answers</th></tr>
<tr><td class="k">custom, reads the Chainlink feed</td><td class="num">16</td><td class="num">$852k</td><td>Friday's print</td></tr>
<tr><td class="k">Morpho ChainlinkOracleV2</td><td class="num">52</td><td class="num">$27.1k</td><td>Friday's print, no staleness check</td></tr>
<tr><td class="k">raw Uniswap pool price</td><td class="num">15</td><td class="num">$2.0k</td><td>the pool, no band</td></tr></table>
<div class="note">Two small custom oracles allow a four-day-old print. Outside these markets PARE accepts a <b>five-day-old</b> one, and already trusts a 30-minute pool TWAP for its other leg.</div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">The product · one contract per asset · <b>drop-in</b> for a Chainlink address</div>
<h2>AfterHours sits in front of the feed.</h2>
<div class="boxes">
  <div class="stack">
    <div class="box"><div class="t">Chainlink feed</div>24/5 · last print per share</div>
    <div class="box"><div class="t">Uniswap v3 pool</div>fixed venue · 24/7 TWAP</div>
    <div class="box"><div class="t">Stock token</div>pause flag · multiplier</div>
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
  <div><b>Quiet feed</b> → median 10-min pool price, <span class="m">±1% first day, ±10% after</span>.</div>
  <div><b>Thin pool</b> (3-window median) → refuse, never switch venue.</div>
  <div><b>Paused, split since print, or print &gt;5 days</b> → refuse.</div>
</div>
<div class="grow"></div>""")

    S.append(demo_slide())

    S.append("""<div class="kicker">Contract quality · <b>github.com/bongbongcrypto/afterhours-oracle</b></div>
<h2>Rust on Arbitrum Stylus. No owner, no upgrade, every number traceable.</h2>
<div class="cols">
  <div class="col"><div class="t">73 tests + 86 on-chain assertions</div><p>Unit and property tests with exact calldata mocks; the real wasm deployed on a local Arbitrum node (ArbOS 61), every session, both bands, a one-window spike, the venue rule and a stock split asserted, gas per read measured.</p></div>
  <div class="col"><div class="t">Tick math vs 80-digit references</div><p>1.0001^tick in Q96 with 512-bit intermediates, checked against independently computed vectors — no magic constants.</p></div>
  <div class="col"><div class="t">Eight review rounds folded in</div><p>Anchor-age cap, a fixed venue an attacker cannot redirect, price and liquidity judged over three sub-windows, a narrow band only while the exchange is open, the share multiplier through dividends and splits. 14 stocks meet the bar today, 28 deployable.</p></div>
</div>
<div><span class="pill on">cargo stylus check ✓ about 39 KB</span><span class="pill">clippy −D warnings ✓</span><span class="pill">AggregatorV3 + Morpho IOracle</span><span class="pill">USDG quote</span></div>
<div class="grow"></div>""")

    S.append("""<div class="kicker">What it unlocks</div>
<h2>Lending that can liquidate on Saturday.<br>Automation that runs seven days.<br>One answer to what a stock is worth while the exchange is closed.</h2>
<div class="grow"></div>
<div class="sub">AfterHours — <span class="mono" style="color:var(--fog)">github.com/bongbongcrypto/afterhours-oracle</span></div>""")
    return S


def render(html_only):
    OUT.mkdir(parents=True, exist_ok=True)
    S = slides()
    for i, body in enumerate(S, 1):
        html = OUT / f"slide-{i}.html"
        io.open(html, "w", encoding="utf-8", newline="\n").write(slide(i, len(S), body))
    print("html: %d slides in %s" % (len(S), OUT))
    if html_only:
        return
    if not os.path.exists(EDGE):
        sys.exit("Edge not found at %s" % EDGE)
    sizes = set()
    for i in range(1, len(S) + 1):
        html = OUT / f"slide-{i}.html"
        png = OUT / f"slide-{i}.png"
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
            sys.exit("capture failed for slide %d (%s)" % (i, png))
        size = png.stat().st_size
        assert size not in sizes, "two slides rendered to byte-identical PNGs (cached page?)"
        sizes.add(size)
        print("  slide-%d.png  %d bytes" % (i, size))


if __name__ == "__main__":
    if "--capture-page" in sys.argv:
        capture_page(sys.argv[sys.argv.index("--capture-page") + 1])
    render(html_only="--html" in sys.argv)
