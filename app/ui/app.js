// Detour window UI. The Rust core measures paths, picks one and gives the
// verdict; this file draws the one screen a player sees. The race table, the
// raw numbers and the relay names live behind "Show details".
(function () {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const ease = (t) => 1 - Math.pow(1 - t, 3);

  const icon = {
    min: '<svg viewBox="0 0 10 10"><path d="M0 5.5h10"/></svg>',
    max: '<svg viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9"/></svg>',
    close: '<svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10"/></svg>',
    x: '<svg class="ico" viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8"/></svg>',
    check: '<svg class="ico" viewBox="0 0 16 16"><path d="M3.5 8.5l3 3 6-7"/></svg>',
    down: '<svg class="ico" viewBox="0 0 16 16"><path d="M6 4l4 4-4 4"/></svg>',
    to: '<svg class="to arr" viewBox="0 0 16 16"><path d="M2.5 8h10.5M9 4l4 4-4 4"/></svg>',
    toSmall: '<svg class="to" viewBox="0 0 16 16"><path d="M2.5 8h10.5M9 4l4 4-4 4"/></svg>',
    paths: '<svg class="ico" viewBox="0 0 16 16"><circle cx="3.5" cy="12" r="1.8"/><circle cx="12.5" cy="4" r="1.8"/><path d="M5.3 12H9a3.5 3.5 0 0 0 3.5-3.5V5.8"/></svg>',
  };
  // The mark: an orange line that takes the corner between two stations,
  // past the dashed straight line it beat.
  const mark = `<svg viewBox="8 8 48 48"><path d="M17 47L47 17" stroke="#a5a7ad" stroke-width="3.6" stroke-dasharray="0.1 7" stroke-linecap="round" fill="none"/><path d="M17 47H35a12 12 0 0 0 12-12V17" stroke="#ff5f1f" stroke-width="8" stroke-linecap="round" fill="none"/><circle cx="17" cy="47" r="5.8" fill="#fff" stroke="#16171a" stroke-width="3.6"/><circle cx="47" cy="17" r="5.8" fill="#fff" stroke="#16171a" stroke-width="3.6"/></svg>`;

  // ---------------------------------------------------------------------------
  // The session: a player in Kraków on 5 GHz Wi-Fi, VALORANT on a Paris server.
  const RACE = [
    { name: 'Direct', sub: 'your normal path', ping: 74, jitter: 18, loss: 2.1, cls: 'direct' },
    { name: 'via waw-1', sub: 'Warsaw', ping: 69, jitter: 14, loss: 1.2 },
    { name: 'via vie-1', sub: 'Vienna', ping: 58, jitter: 6, loss: 0.3 },
    { name: 'via fra-2, par-1', sub: 'Frankfurt', ping: 46, jitter: 3, loss: 0.3, cls: 'best' },
  ];

  // ---------------------------------------------------------------------------
  // The picture: your normal path in grey, Detour's path in orange. Nothing else.
  const YOU = [110, 116], SRV = [610, 116];
  const LINE = {
    direct: 'M110 116H610',
    path: 'M110 116L170 56H550L610 116',
    low: 'M110 116L170 176H550L610 116',
  };
  function pill(x, y, text, cls = '') {
    const w = text.length * 7.6 + 18;
    return `<g class="ms ${cls}"><rect x="${x - w / 2}" y="${y - 12}" width="${w}" height="24" rx="12"/><text x="${x}" y="${y + 4.5}" text-anchor="middle">${text}</text></g>`;
  }
  function picture({ from = 'Kraków', to = 'Paris server', via = 'Frankfurt' } = {}) {
    return `
    <svg viewBox="0 0 720 214">
      <path class="cand fade" id="p-low" d="${LINE.low}"/>
      <text class="clbl fade" id="p-low-l" x="360" y="204" text-anchor="middle">Vienna</text>
      <path class="cand fade" id="p-top" d="${LINE.path}"/>
      <path class="direct" d="${LINE.direct}"/>
      <path class="path" id="p-path" d="${LINE.path}" style="opacity:0"/>
      <g class="fade" id="p-via">
        <circle class="stn" cx="300" cy="56" r="7"/>
        <text class="lbl" x="300" y="34" text-anchor="middle">${via}</text>
        ${pill(450, 56, '46 ms', 'hot')}
      </g>
      ${pill(360, 116, '74 ms')}
      <g id="probes"></g>
      <circle class="end" cx="${YOU[0]}" cy="${YOU[1]}" r="10"/><circle class="dot" cx="${YOU[0]}" cy="${YOU[1]}" r="3.5"/>
      <text class="lbl" x="${YOU[0] - 20}" y="${YOU[1] - 2}" text-anchor="end">You</text>
      <text class="lsub" x="${YOU[0] - 20}" y="${YOU[1] + 15}" text-anchor="end">${from}</text>
      <circle class="end" cx="${SRV[0]}" cy="${SRV[1]}" r="10"/><circle class="dot" cx="${SRV[0]}" cy="${SRV[1]}" r="3.5"/>
      <text class="lbl" x="${SRV[0] + 20}" y="${SRV[1] - 2}">${to.split(' ')[0]}</text>
      <text class="lsub" x="${SRV[0] + 20}" y="${SRV[1] + 15}">game server</text>
    </svg>`;
  }

  // ---------------------------------------------------------------------------
  function shell(view) {
    return `
    <div class="win">
      <header class="titlebar">
        <div class="brand">${mark}<span class="wordmark">Detour</span></div>
        <button class="paths-btn" id="paths-btn">${icon.paths}Paths</button>
        <div class="winctl"><span>${icon.min}</span><span>${icon.max}</span><span>${icon.close}</span></div>
      </header>
      <main class="view" id="view">${view}</main>
    </div>`;
  }

  function mainView() {
    return `
    <section class="stage idle" id="stage">
      <div class="idle-msg">${mark}<h1>Start a game. We’ll pick it up.</h1></div>
      <div class="game">
        <div class="chip"><i class="tile">VA</i><b>VALORANT is running</b><span>· Paris server</span></div>
        <div class="big"><span class="was-wrap"><span class="was">74</span>${icon.to}</span><span class="now" id="now">74</span><span class="u">ms</span></div>
        <p class="say" id="say">Your ping to the Paris server right now.</p>
        <p class="sub spikes">Lag spikes: <b>11 a minute</b>${icon.toSmall}<b>0</b></p>
        <div class="pic" id="pic">${picture()}</div>
        <button class="cta" id="cta"><i class="spin"></i><span id="cta-l">Lower my ping</span><span class="switch on"></span></button>
        <button class="more" id="more">Show details${icon.down}</button>
      </div>
      <p class="foot">Only VALORANT’s traffic takes this path. Everything else goes as usual.</p>
    </section>`;
  }

  // ---------------------------------------------------------------------------
  // Show details: the race, the lag spikes and the technical names.
  function rng(seed) {
    return () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
  }
  function series(base, wobble, seed) {
    const r = rng(seed);
    let v = base;
    return Array.from({ length: 240 }, () => {
      v += (base - v) * 0.35 + (r() - 0.5) * wobble;
      return v;
    });
  }
  function spikeGraph() {
    const x0 = 0, x1 = 418, sx = (i) => x0 + (i / 239) * (x1 - x0);
    const strip = (top) => (ms) => top + 54 - Math.min(200, ms) * 0.27;
    const yA = strip(18), yB = strip(104);
    const direct = series(74, 26, 7);
    const spikes = [18, 47, 83, 109, 151, 204];
    const spikeMs = [138, 171, 126, 186, 149, 162];
    const lost = [33, 96, 127, 176, 221];
    spikes.forEach((i, k) => { direct[i] = spikeMs[k]; direct[i + 1] = (direct[i] + 74) / 2; });
    const after = series(46, 5, 11);
    const line = (vals, y, skip = []) => {
      let d = '', pen = false;
      vals.forEach((v, i) => {
        if (skip.includes(i)) { pen = false; return; }
        d += `${pen ? 'L' : 'M'}${sx(i).toFixed(1)} ${y(v).toFixed(1)}`;
        pen = true;
      });
      return d;
    };
    return `<svg viewBox="0 0 418 160">
      <text class="sp-k" x="0" y="11">Normal path</text><text class="sp-v" x="418" y="11" text-anchor="end" fill="#d23a3a">11 spikes · 2.1% lost</text>
      <line class="g-grid" x1="0" x2="418" y1="${yA(0)}" y2="${yA(0)}"/>
      <path class="g-direct" d="${line(direct, yA, lost)}"/>
      ${spikes.map((i) => `<circle class="g-spike" cx="${sx(i)}" cy="${yA(direct[i])}" r="2.6"/>`).join('')}
      ${lost.map((i) => `<line class="g-loss" x1="${sx(i)}" x2="${sx(i)}" y1="${yA(196)}" y2="${yA(176)}"/>`).join('')}
      <text class="sp-k" x="0" y="97">With Detour</text><text class="sp-v" x="418" y="97" text-anchor="end" fill="#1f8a5b">0 spikes · 0.0% lost</text>
      <line class="g-grid" x1="0" x2="418" y1="${yB(0)}" y2="${yB(0)}"/>
      <path class="g-route" d="${line(after, yB)}"/>
    </svg>`;
  }
  function detailsDrawer() {
    return `
    <div class="scrim"></div>
    <aside class="drawer" id="drawer">
      <div class="sheet-h"><div><h2>Details</h2><p>VALORANT · Kraków to the Paris server</p></div><span class="x">${icon.x}</span></div>
      <div class="sec">
        <h3>Path race<span>every 5 seconds</span></h3>
        <table class="rt">
          <tr><th>Path</th><th>Ping</th><th>Jitter</th><th>Loss</th></tr>
          ${RACE.map((r) => `<tr class="${r.cls || ''}"><td><i></i>${r.name} <em>${r.sub}</em></td><td>${r.ping}<small>ms</small></td><td>${r.jitter}<small>ms</small></td><td>${r.loss.toFixed(1)}<small>%</small></td></tr>`).join('')}
        </table>
        <p class="note">The first race picks the path. After that, Detour switches only when another path leads by 5 ms for 3 races in a row, so it doesn’t flap mid-round.</p>
      </div>
      <div class="sec sp">
        <h3>Lag spikes<span>the last minute</span></h3>
        ${spikeGraph()}
        <p class="note">Multipath: each packet goes two ways to par-1 in Paris, which keeps the first copy. Costs +13 kB/s up, +22 kB/s down.</p>
      </div>
      <p class="tech">Wintun adapter · VALORANT UDP only · signed relay list</p>
    </aside>`;
  }

  // Paths: where faster paths come from. Two choices, and the wizard under the second.
  function pathsSheet() {
    const steps = [['done', 'Free account'], ['done', 'Paste your key'], ['now', 'Deploy']];
    return `
    <div class="scrim"></div>
    <div class="sheet" id="sheet">
      <div class="sheet-h"><div><h2>Paths</h2><p>Where Detour finds faster paths. Both are free.</p></div><span class="x">${icon.x}</span></div>
      <div class="opt" id="opt-community"><i class="radio"></i><b>Free community paths <em>Default</em></b><p>Fast servers run by volunteers in 12 cities. Nothing to set up.</p></div>
      <div class="opt sel" id="opt-own"><i class="radio"></i><b>My own free server</b><p>Yours alone, on Oracle Cloud’s free tier. About 3 minutes.</p>
        <div class="wiz">
          <div class="steps">${steps.map(([s, t], i) => `<div class="step ${s}"><span>${s === 'done' ? icon.check : i + 1}</span>${t}</div>`).join('')}</div>
          <div class="prog"><b>Setting up your server in Frankfurt…</b><div class="bar"><i></i></div><div class="prog-l"><span>Installing the Detour relay</span><span>1:48 of about 3 min</span></div></div>
          <p class="hint">Free servers sometimes run out in busy regions. If that happens, Detour keeps trying on its own.</p>
        </div>
      </div>
      <div class="sheet-f"><button class="done-btn">Done</button></div>
    </div>`;
  }

  // A verdict on its own, with the game it's about: one sentence and one line of advice.
  const VERDICTS = {
    route: { tile: 'VA', game: 'VALORANT', on: true, say: 'Faster path found through Frankfurt.', advice: 'Your normal path takes the long way. This one is 28 ms quicker.' },
    wifi: { tile: 'FN', game: 'Fortnite', on: false, say: 'Your Wi-Fi is the problem.', advice: 'A faster path won’t help. Use a cable or 5 GHz Wi-Fi.' },
    valve: { tile: 'CS', game: 'Counter-Strike 2', on: false, say: 'This game already has a fast path.', advice: 'Valve routes CS2 through its own network. Don’t pay anyone for a ping booster.' },
  };
  function verdictCard(kind) {
    const v = VERDICTS[kind];
    const fig = {
      route: `<span class="was">74</span>${icon.to}<span class="now">46</span><span class="u">ms</span>`,
      wifi: `<svg viewBox="0 0 330 70"><path d="M24 32H165" stroke="#d23a3a" stroke-width="5" stroke-linecap="round"/><path d="M165 32H306" stroke="#a5a7ad" stroke-width="3" stroke-dasharray="2 8" stroke-linecap="round"/>
        ${[[24, 'You'], [165, 'Wi-Fi'], [306, 'Internet']].map(([x, t]) => `<circle cx="${x}" cy="32" r="8" fill="#fff" stroke="#16171a" stroke-width="3"/><text x="${x}" y="62" text-anchor="middle" style="font-size:13px;font-weight:600;fill:#16171a">${t}</text>`).join('')}
        <text x="94" y="20" text-anchor="middle" style="font-size:12px;font-weight:600;fill:#d23a3a">3.4% lost</text></svg>`,
      valve: `<span class="now">31</span><span class="u">ms</span>`,
    }[kind];
    return `
    <article class="vcard">
      <header><i class="tile">${v.tile}</i><b>${v.game}</b><span class="st ${v.on ? 'on' : ''}"><i></i>${v.on ? 'Detour on' : 'Detour stays off'}</span></header>
      <div class="vfig ${kind === 'wifi' ? '' : 'num'}">${fig}</div>
      <h3>${v.say}</h3>
      <p>${v.advice}</p>
    </article>`;
  }

  // ---------------------------------------------------------------------------
  const root = document.getElementById('app');
  const stage = () => $('#stage');
  const fade = (sel, on = true) => $(sel)?.classList.toggle('on', on);

  function setState(s) {
    stage().className = `stage ${s}`;
  }
  function setOff() {
    setState('off');
    $('#now').textContent = '74';
    $('#say').textContent = 'Your ping to the Paris server right now.';
    $('#cta-l').textContent = 'Lower my ping';
  }
  function setOn() {
    setState('on');
    $('#now').textContent = '46';
    $('#say').textContent = 'Faster path found through Frankfurt.';
    $('#cta-l').textContent = 'Detour is on';
    $('#p-path').style.opacity = 1;
    fade('#p-via');
    ['#p-low', '#p-low-l', '#p-top'].forEach((s) => fade(s, false));
  }

  // Probe dots run each path; a slower path's dots take longer.
  function runProbes(ms) {
    const g = $('#probes');
    const paths = [['direct', 74], ['low', 58], ['path', 46]].map(([id, ping]) => {
      const p = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      p.setAttribute('d', LINE[id]);
      return { p, len: p.getTotalLength(), period: ping * 20 };
    });
    g.innerHTML = paths.map(() => '<circle class="probe" r="4"/><circle class="probe" r="4"/>').join('');
    const dots = $$('circle', g);
    const t0 = performance.now();
    return new Promise((done) => {
      (function frame(now) {
        const t = now - t0;
        paths.forEach((q, i) => {
          [0, 0.5].forEach((off, j) => {
            const pt = q.p.getPointAtLength((((t / q.period) + off) % 1) * q.len);
            const dot = dots[i * 2 + j];
            dot.setAttribute('cx', pt.x.toFixed(1));
            dot.setAttribute('cy', pt.y.toFixed(1));
            dot.style.opacity = t < 200 ? t / 200 : t > ms - 250 ? Math.max(0, (ms - t) / 250) : 1;
          });
        });
        if (t < ms) requestAnimationFrame(frame); else { g.innerHTML = ''; done(); }
      })(t0);
    });
  }

  function drawPath(ms) {
    const w = $('#p-path');
    const len = w.getTotalLength();
    w.style.strokeDasharray = `${len} ${len}`;
    w.style.strokeDashoffset = len;
    w.style.opacity = 1;
    const t0 = performance.now();
    return new Promise((done) => {
      (function frame(now) {
        const t = Math.min(1, (now - t0) / ms);
        w.style.strokeDashoffset = len * (1 - ease(t));
        if (t < 1) requestAnimationFrame(frame); else { w.style.strokeDasharray = ''; done(); }
      })(t0);
    });
  }

  function countDown(ms) {
    const t0 = performance.now();
    return new Promise((done) => {
      (function frame(now) {
        const t = Math.min(1, (now - t0) / ms);
        $('#now').textContent = String(Math.round(74 + (46 - 74) * ease(t)));
        if (t < 1) requestAnimationFrame(frame); else done();
      })(t0);
    });
  }

  async function swapText(sel, text) {
    const el = $(sel);
    el.style.transition = 'opacity 0.2s';
    el.style.opacity = 0;
    await sleep(200);
    el.textContent = text;
    el.style.opacity = 1;
  }

  window.detour = {
    idle() {
      root.innerHTML = shell(mainView());
    },
    // The game starts: Detour sees its exe and learns the server from its traffic.
    async launch() {
      $('#view').insertAdjacentHTML('beforeend', `<div class="toast" id="toast"><i class="tile">VA</i>VALORANT is running. Found its server in Paris.</div>`);
      await sleep(30);
      fade('#toast');
      await sleep(500);
      setOff();
      await sleep(1600);
      fade('#toast', false);
    },
    press(sel) {
      const b = $(sel);
      b.classList.add('pressed');
      return sleep(150).then(() => b.classList.remove('pressed'));
    },
    // The race: dots run every path, the winner draws in orange, the number drops.
    async race(ms = 2400) {
      setState('racing');
      $('#cta-l').textContent = 'Finding a faster path…';
      $('#say').textContent = 'Racing your normal path against faster ones.';
      ['#p-low', '#p-low-l', '#p-top'].forEach((s) => fade(s));
      await sleep(200);
      await runProbes(ms);
      ['#p-low', '#p-low-l', '#p-top'].forEach((s) => fade(s, false));
      await drawPath(650);
      fade('#p-via');
      $('#cta-l').textContent = 'Detour is on';
      setState('on');
      await Promise.all([countDown(900), swapText('#say', 'Faster path found through Frankfurt.')]);
    },
    // Stills.
    result() {
      this.idle();
      setOn();
    },
    details() {
      this.result();
      $('#view').insertAdjacentHTML('beforeend', detailsDrawer());
    },
    paths() {
      this.result();
      $('#paths-btn').classList.add('open');
      $('#view').insertAdjacentHTML('beforeend', pathsSheet());
    },
  };
  window.DetourUI = { picture, verdictCard, mark, icon };

  if (root) window.detour.idle();
})();
