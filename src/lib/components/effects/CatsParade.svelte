<script lang="ts">
  // Premium "Atelier" theme decoration — a tiny pixel-art menagerie that
  // walks across the bottom edge of the viewport on top of the StatusBar.
  //
  // Each animal:
  //   - drawn inline as SVG (no sprite assets, no audio files)
  //   - faces its direction of travel (art is drawn LEFT-facing in source
  //     and the inner <g class="cn-flip"> gets `scaleX(-1)` for right-walkers,
  //     leaving the outer wrapper's transform free for translateX/translateY)
  //   - is clickable: click → vertical jump + synthesized voice
  //   - on a randomised 10-25s timer pauses, plays a kind-specific INTERACT
  //     animation (cat claws / dog crouch / bunny duck / bird peck), drops a
  //     kind-specific LEFTOVER (claw marks / paw print / fur tuft / feather)
  //     at its current screen position, then resumes walking. Leftovers
  //     fade and disappear after ~4.5s.
  //
  // Visibility is bound to the persisted appearance.theme — non-Pro
  // previewers see the warm palette but not the parade (the parade is
  // the upgrade carrot).
  //
  // Respects `prefers-reduced-motion: reduce` (parade is hidden entirely).
  import { onDestroy } from "svelte";
  import { appearance } from "$lib/stores/settings";

  type Dir = "left" | "right";
  type Kind = "cat" | "dog" | "bunny" | "bird";
  type Animal = {
    kind: Kind;
    color: string;
    eye: string;
    accent?: string;
    dur: number;     // seconds to cross viewport
    delay: number;   // start delay (s) so they don't bunch
    step: number;    // walk-cycle frame swap interval (s)
    dir: Dir;
  };

  const animals: Animal[] = [
    { kind: "cat",   color: "#e09b58", eye: "#a8c275", dur: 55, delay: 0,  step: 0.42, dir: "right" },
    { kind: "cat",   color: "#8a8a92", eye: "#7dcfff", dur: 78, delay: 14, step: 0.52, dir: "left"  },
    { kind: "cat",   color: "#1c1814", eye: "#f6c177", dur: 44, delay: 27, step: 0.36, dir: "right" },
    { kind: "dog",   color: "#c9925a", eye: "#1c1814", dur: 50, delay: 8,  step: 0.40, dir: "left"  },
    { kind: "bunny", color: "#f0e8d8", eye: "#d97462", dur: 60, delay: 35, step: 0.45, dir: "right" },
    { kind: "bird",  color: "#e6c75a", eye: "#1c1814", accent: "#d97462", dur: 35, delay: 20, step: 0.25, dir: "left" },
  ];

  let active = $derived($appearance.theme === "atelier");

  // ─── Click jump ───────────────────────────────────────────────────────
  let jumping = $state<Set<number>>(new Set());

  function poke(idx: number, kind: Kind) {
    jumping.add(idx);
    jumping = new Set(jumping);
    window.setTimeout(() => {
      jumping.delete(idx);
      jumping = new Set(jumping);
    }, 600);
    playVoice(kind);
  }

  // ─── Interact cycle + leftovers ───────────────────────────────────────
  // Each animal independently schedules its next interact 10-25s after the
  // previous one ends. During interact: walk is paused (animation-play-state)
  // and the kind-specific keyframe plays on the inner SVG. At the peak of
  // the interact we capture the animal's current screen position and spawn
  // a kind-specific leftover SVG that lives ~4.5s before fading.
  type Leftover = { id: number; kind: Kind; x: number; y: number };
  let interacting = $state<Set<number>>(new Set());
  let leftovers = $state<Leftover[]>([]);
  let leftoverSeq = 0;

  const INTERACT_MIN_GAP_MS = 10_000;
  const INTERACT_MAX_GAP_MS = 25_000;
  const INTERACT_DUR_MS = 1200;
  const LEFTOVER_TTL_MS = 4500;

  $effect(() => {
    if (!active) return;
    const timers: number[] = [];

    function schedule(idx: number) {
      const wait =
        INTERACT_MIN_GAP_MS +
        Math.random() * (INTERACT_MAX_GAP_MS - INTERACT_MIN_GAP_MS);
      timers.push(window.setTimeout(() => trigger(idx), wait));
    }

    function trigger(idx: number) {
      if (!active) return;
      const buttons = document.querySelectorAll<HTMLElement>(".cn-anim");
      const btn = buttons[idx];
      // Skip + reschedule if the animal is off-screen so the leftover lands
      // somewhere the user can actually see.
      if (!btn) {
        schedule(idx);
        return;
      }
      const r = btn.getBoundingClientRect();
      if (r.right < 20 || r.left > window.innerWidth - 20) {
        schedule(idx);
        return;
      }

      interacting.add(idx);
      interacting = new Set(interacting);

      // Spawn the leftover slightly behind the animal (opposite to travel
      // direction) at the peak of the interact animation — reads as "the
      // animal just did something at that spot."
      const kind = animals[idx].kind;
      const dir = animals[idx].dir;
      timers.push(
        window.setTimeout(() => {
          const r2 = btn.getBoundingClientRect();
          const cx = r2.left + r2.width / 2;
          // Offset behind the animal: ~12px in the opposite direction.
          const behindX = cx + (dir === "right" ? -12 : 12);
          const groundY = r2.bottom - 4;
          const id = ++leftoverSeq;
          leftovers = [...leftovers, { id, kind, x: behindX, y: groundY }];
          timers.push(
            window.setTimeout(() => {
              leftovers = leftovers.filter((l) => l.id !== id);
            }, LEFTOVER_TTL_MS)
          );
        }, INTERACT_DUR_MS * 0.55)
      );

      // End the interact + reschedule the next one.
      timers.push(
        window.setTimeout(() => {
          interacting.delete(idx);
          interacting = new Set(interacting);
          schedule(idx);
        }, INTERACT_DUR_MS)
      );
    }

    // Stagger the very first interact per animal so they don't all stop at
    // once — initial wait is 4-12s rather than the full 10-25s range.
    for (let i = 0; i < animals.length; i++) {
      timers.push(
        window.setTimeout(
          () => trigger(i),
          4_000 + Math.random() * 8_000 + i * 1_500
        )
      );
    }

    return () => {
      for (const t of timers) window.clearTimeout(t);
      interacting = new Set();
      leftovers = [];
    };
  });

  // ─── Synthesized voices (no audio file assets) ────────────────────────
  let audioCtx: AudioContext | null = null;
  function ensureAudioCtx(): AudioContext | null {
    if (typeof window === "undefined") return null;
    if (!audioCtx) {
      const AC =
        window.AudioContext ||
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!AC) return null;
      audioCtx = new AC();
    }
    if (audioCtx.state === "suspended") audioCtx.resume().catch(() => {});
    return audioCtx;
  }
  // Defensive teardown — browsers cap the number of AudioContext instances
  // per page (~6 in Chromium). This component lives at the app root so it
  // typically only unmounts on app close, but HMR / dev rebuilds + a future
  // refactor where the parade is conditionally mounted could leak contexts
  // without this.
  onDestroy(() => {
    if (audioCtx && audioCtx.state !== "closed") {
      audioCtx.close().catch(() => {});
      audioCtx = null;
    }
  });
  function playVoice(kind: Kind) {
    const ctx = ensureAudioCtx();
    if (!ctx) return;
    switch (kind) {
      case "cat":   playMeow(ctx);   break;
      case "dog":   playWoof(ctx);   break;
      case "bunny": playSqueak(ctx); break;
      case "bird":  playChirp(ctx);  break;
    }
  }
  // Cat "mee-ow" — multi-formant synthesis aiming for a cartoon cat sound.
  // (A real recorded sample would always sound more authentic; if you drop
  //  an mp3/ogg/wav at `static/sounds/cat.mp3` ask me to wire up sample
  //  playback as a primary path with this synthesis as the fallback.)
  //
  // Construction:
  //   1. 40ms low-passed noise burst → simulates the "m" consonant onset
  //      (mouth closed, vibrating nasal-cavity buzz).
  //   2. Sawtooth carrier with pitch contour 360 → 580 → 250 Hz across 750ms
  //      — the rise-then-fall is what gives "meow" its arc.
  //   3. 6 Hz sine LFO modulating the carrier frequency ±18 Hz → vibrato
  //      that makes the tone read as biological rather than electronic.
  //   4. Two parallel band-pass filters acting as vowel formants F1/F2,
  //      both sweeping from "ee"-like values (high) at the start to "ow"-
  //      like values (low) at the end. This formant sweep is what actually
  //      sells the "mee → ow" vowel transition in the human ear; without
  //      it, any pitched tone is just a beep.
  //   5. Gentle 3.5 kHz low-pass keeps the sawtooth from feeling buzzy.
  function playMeow(ctx: AudioContext) {
    const t = ctx.currentTime;
    const dur = 0.75;

    // (1) Noise burst — the "m" onset.
    const noiseBuf = ctx.createBuffer(1, Math.floor(ctx.sampleRate * 0.04), ctx.sampleRate);
    const data = noiseBuf.getChannelData(0);
    for (let i = 0; i < data.length; i++) {
      // Decaying white noise: full amplitude → silence over 40ms.
      data[i] = (Math.random() * 2 - 1) * (1 - i / data.length);
    }
    const noise = ctx.createBufferSource();
    noise.buffer = noiseBuf;
    const noiseLp = ctx.createBiquadFilter();
    noiseLp.type = "lowpass";
    noiseLp.frequency.value = 420;
    const noiseGain = ctx.createGain();
    noiseGain.gain.value = 0.10;
    noise.connect(noiseLp).connect(noiseGain).connect(ctx.destination);
    noise.start(t);

    // (2) Carrier — sawtooth with meow-shaped pitch contour.
    const osc = ctx.createOscillator();
    osc.type = "sawtooth";
    osc.frequency.setValueAtTime(360, t);
    osc.frequency.linearRampToValueAtTime(580, t + 0.18);
    osc.frequency.exponentialRampToValueAtTime(250, t + dur);

    // (3) Vibrato — 6 Hz LFO into carrier frequency.
    const vibrato = ctx.createOscillator();
    vibrato.type = "sine";
    vibrato.frequency.value = 6;
    const vibratoDepth = ctx.createGain();
    vibratoDepth.gain.value = 18;
    vibrato.connect(vibratoDepth).connect(osc.frequency);

    // (5) Pre-formant brightness tame.
    const tilt = ctx.createBiquadFilter();
    tilt.type = "lowpass";
    tilt.frequency.value = 3500;

    // (4) F1 / F2 formants sweeping ee → ow.
    const f1 = ctx.createBiquadFilter();
    f1.type = "bandpass";
    f1.frequency.setValueAtTime(780, t);
    f1.frequency.linearRampToValueAtTime(560, t + dur);
    f1.Q.value = 10;

    const f2 = ctx.createBiquadFilter();
    f2.type = "bandpass";
    f2.frequency.setValueAtTime(2200, t);
    f2.frequency.linearRampToValueAtTime(1050, t + dur);
    f2.Q.value = 12;

    // Envelope — handoff from noise burst at ~40ms.
    const env = ctx.createGain();
    env.gain.setValueAtTime(0, t + 0.03);
    env.gain.linearRampToValueAtTime(0.22, t + 0.08);
    env.gain.setValueAtTime(0.20, t + 0.45);
    env.gain.exponentialRampToValueAtTime(0.001, t + dur);

    // Sum F1 + F2 in parallel and run through envelope.
    const sum = ctx.createGain();
    osc.connect(tilt);
    tilt.connect(f1).connect(sum);
    tilt.connect(f2).connect(sum);
    sum.connect(env).connect(ctx.destination);

    osc.start(t);    osc.stop(t + dur + 0.05);
    vibrato.start(t); vibrato.stop(t + dur + 0.05);
  }
  function playWoof(ctx: AudioContext) {
    const t = ctx.currentTime;
    const osc = ctx.createOscillator(); const gain = ctx.createGain();
    osc.type = "square";
    osc.frequency.setValueAtTime(220, t);
    osc.frequency.exponentialRampToValueAtTime(85, t + 0.14);
    gain.gain.setValueAtTime(0, t);
    gain.gain.linearRampToValueAtTime(0.14, t + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.001, t + 0.18);
    osc.connect(gain).connect(ctx.destination);
    osc.start(t); osc.stop(t + 0.2);
  }
  function playSqueak(ctx: AudioContext) {
    const t = ctx.currentTime;
    const osc = ctx.createOscillator(); const gain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.setValueAtTime(1200, t);
    osc.frequency.linearRampToValueAtTime(1700, t + 0.04);
    osc.frequency.linearRampToValueAtTime(900, t + 0.13);
    gain.gain.setValueAtTime(0, t);
    gain.gain.linearRampToValueAtTime(0.09, t + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.001, t + 0.16);
    osc.connect(gain).connect(ctx.destination);
    osc.start(t); osc.stop(t + 0.17);
  }
  function playChirp(ctx: AudioContext) {
    for (let i = 0; i < 2; i++) {
      const t = ctx.currentTime + i * 0.09;
      const osc = ctx.createOscillator(); const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.setValueAtTime(2100, t);
      osc.frequency.exponentialRampToValueAtTime(3600, t + 0.05);
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(0.07, t + 0.008);
      gain.gain.exponentialRampToValueAtTime(0.001, t + 0.06);
      osc.connect(gain).connect(ctx.destination);
      osc.start(t); osc.stop(t + 0.08);
    }
  }
</script>

{#if active}
  <!--
    Container is intentionally NOT aria-hidden — the <button> children
    inside are real interactive elements (poke → jump + voice) with
    aria-labels, and hiding the wrapper would silently break screen-reader
    discoverability while leaving the buttons keyboard-focusable.
    Decorative children (leftover SVGs) carry aria-hidden individually.
  -->
  <div class="cn-parade">
    {#each animals as anim, i}
      <button
        type="button"
        class="cn-anim cn-anim-{anim.kind}"
        class:jumping={jumping.has(i)}
        class:interacting={interacting.has(i)}
        data-dir={anim.dir}
        style="--cn-color:{anim.color}; --cn-eye:{anim.eye}; --cn-accent:{anim.accent ?? anim.color}; --cn-dur:{anim.dur}s; --cn-delay:{anim.delay}s; --cn-step:{anim.step}s;"
        onclick={() => poke(i, anim.kind)}
        aria-label="Pet the {anim.kind}"
      >
        {#if anim.kind === "cat"}
          <svg class="cn-svg" viewBox="0 0 28 24" width="28" height="24">
            <g class="cn-flip">
              <polygon points="3,6 5,1 7,6" fill="var(--cn-color)" />
              <polygon points="8,6 10,1 12,6" fill="var(--cn-color)" />
              <rect x="2" y="6" width="11" height="8" rx="1" fill="var(--cn-color)" />
              <rect x="9.5" y="9" width="1.5" height="2" fill="var(--cn-eye)" />
              <rect x="9" y="11" width="14" height="7" rx="1" fill="var(--cn-color)" />
              <path d="M 22 12 Q 28 9 26 4" stroke="var(--cn-color)" stroke-width="2" fill="none" stroke-linecap="round" />
              <g class="cn-legs cn-legs-a">
                <rect x="10" y="18" width="2" height="4" fill="var(--cn-color)" />
                <rect x="18" y="18" width="2" height="4" fill="var(--cn-color)" />
              </g>
              <g class="cn-legs cn-legs-b">
                <rect x="9" y="18" width="2" height="4" fill="var(--cn-color)" />
                <rect x="20" y="18" width="2" height="4" fill="var(--cn-color)" />
              </g>
              <!-- Front paw used during interact (claw swipe). Hidden by
                   default; .interacting toggles a swipe keyframe. -->
              <g class="cn-paw">
                <rect x="-1" y="15" width="3" height="5" rx="0.5" fill="var(--cn-color)" />
                <rect x="-2" y="14" width="1" height="2" fill="var(--cn-eye)" />
                <rect x="-2" y="17" width="1" height="2" fill="var(--cn-eye)" />
              </g>
            </g>
          </svg>
        {:else if anim.kind === "dog"}
          <svg class="cn-svg" viewBox="0 0 32 24" width="32" height="24">
            <g class="cn-flip">
              <polygon points="2,7 5,15 7,7" fill="var(--cn-color)" />
              <polygon points="9,7 12,15 13,7" fill="var(--cn-color)" />
              <rect x="2" y="6" width="11" height="8" rx="2" fill="var(--cn-color)" />
              <rect x="0" y="9" width="3" height="4" rx="1" fill="var(--cn-color)" />
              <rect x="0.5" y="10.5" width="1" height="1" fill="var(--cn-eye)" />
              <rect x="6" y="9" width="1.5" height="2" fill="var(--cn-eye)" />
              <rect x="9" y="11" width="18" height="7" rx="2" fill="var(--cn-color)" />
              <!-- Tail wags faster during interact via separate animation. -->
              <rect class="cn-tail" x="26" y="7" width="2" height="6" rx="1" fill="var(--cn-color)" />
              <g class="cn-legs cn-legs-a">
                <rect x="11" y="18" width="2" height="4" fill="var(--cn-color)" />
                <rect x="23" y="18" width="2" height="4" fill="var(--cn-color)" />
              </g>
              <g class="cn-legs cn-legs-b">
                <rect x="10" y="18" width="2" height="4" fill="var(--cn-color)" />
                <rect x="25" y="18" width="2" height="4" fill="var(--cn-color)" />
              </g>
            </g>
          </svg>
        {:else if anim.kind === "bunny"}
          <svg class="cn-svg cn-svg-bunny" viewBox="0 0 24 28" width="24" height="28">
            <g class="cn-flip">
              <rect x="6" y="0" width="3" height="10" rx="1.5" fill="var(--cn-color)" />
              <rect x="11" y="0" width="3" height="10" rx="1.5" fill="var(--cn-color)" />
              <rect x="4" y="9" width="10" height="8" rx="3" fill="var(--cn-color)" />
              <rect x="11" y="12" width="1.5" height="2" fill="var(--cn-eye)" />
              <rect x="9" y="14" width="11" height="9" rx="3" fill="var(--cn-color)" />
              <circle cx="20.5" cy="18" r="2" fill="var(--cn-color)" />
              <rect x="14" y="22" width="6" height="3" rx="1.5" fill="var(--cn-color)" />
              <rect x="9" y="23" width="3" height="2" rx="1" fill="var(--cn-color)" />
            </g>
          </svg>
        {:else if anim.kind === "bird"}
          <svg class="cn-svg" viewBox="0 0 18 18" width="18" height="18">
            <g class="cn-flip">
              <circle cx="9" cy="9" r="6" fill="var(--cn-color)" />
              <!-- Beak gets its own group so it can be tilted-down during
                   the peck keyframe. -->
              <g class="cn-beak">
                <polygon points="2,9 -1,10 2,11" fill="var(--cn-accent)" />
              </g>
              <circle cx="6" cy="7" r="1" fill="var(--cn-eye)" />
              <ellipse cx="11" cy="10" rx="3" ry="2" fill="var(--cn-color)" opacity="0.55" />
              <g class="cn-legs cn-legs-a">
                <rect x="7" y="14" width="1" height="3" fill="var(--cn-accent)" />
                <rect x="10" y="14" width="1" height="3" fill="var(--cn-accent)" />
              </g>
              <g class="cn-legs cn-legs-b">
                <rect x="6" y="14" width="1" height="3" fill="var(--cn-accent)" />
                <rect x="11" y="14" width="1" height="3" fill="var(--cn-accent)" />
              </g>
            </g>
          </svg>
        {/if}
      </button>
    {/each}

    <!-- Leftovers — fixed-positioned at the spot the animal was at the peak
         of its interact. Fade + drift down over LEFTOVER_TTL_MS. -->
    {#each leftovers as lo (lo.id)}
      <div
        class="cn-leftover cn-leftover-{lo.kind}"
        style="left:{lo.x}px; top:{lo.y}px;"
        aria-hidden="true"
      >
        {#if lo.kind === "cat"}
          <!-- Three diagonal claw marks. -->
          <svg viewBox="0 0 16 10" width="16" height="10">
            <line x1="1" y1="1" x2="15" y2="3"   stroke="rgba(245,235,217,0.75)" stroke-width="1.2" stroke-linecap="round" />
            <line x1="1" y1="5" x2="15" y2="6.5" stroke="rgba(245,235,217,0.72)" stroke-width="1.2" stroke-linecap="round" />
            <line x1="1" y1="9" x2="15" y2="9.5" stroke="rgba(245,235,217,0.68)" stroke-width="1.2" stroke-linecap="round" />
          </svg>
        {:else if lo.kind === "dog"}
          <!-- Paw print: one pad + four toes. -->
          <svg viewBox="0 0 14 12" width="14" height="12">
            <ellipse cx="7" cy="8.5" rx="3.4" ry="2.6" fill="rgba(245,235,217,0.62)" />
            <circle cx="3" cy="4" r="1.3" fill="rgba(245,235,217,0.62)" />
            <circle cx="6" cy="2.5" r="1.3" fill="rgba(245,235,217,0.62)" />
            <circle cx="9" cy="2.5" r="1.3" fill="rgba(245,235,217,0.62)" />
            <circle cx="11.5" cy="4" r="1.3" fill="rgba(245,235,217,0.62)" />
          </svg>
        {:else if lo.kind === "bunny"}
          <!-- Tiny fur tufts. -->
          <svg viewBox="0 0 14 10" width="14" height="10">
            <path d="M 2 8 Q 4 1 6 8" stroke="rgba(245,235,217,0.78)" stroke-width="1.2" fill="none" stroke-linecap="round" />
            <path d="M 6 9 Q 8 2 10 9" stroke="rgba(245,235,217,0.78)" stroke-width="1.2" fill="none" stroke-linecap="round" />
            <path d="M 9 8 Q 11 2 13 8" stroke="rgba(245,235,217,0.75)" stroke-width="1.2" fill="none" stroke-linecap="round" />
          </svg>
        {:else if lo.kind === "bird"}
          <!-- Single dropped feather + tiny peck dot. -->
          <svg viewBox="0 0 12 12" width="12" height="12">
            <ellipse cx="6" cy="6" rx="2.4" ry="4" transform="rotate(35 6 6)" fill="rgba(230,199,90,0.75)" />
            <line x1="6" y1="2" x2="6" y2="10" transform="rotate(35 6 6)" stroke="rgba(217,116,98,0.55)" stroke-width="0.6" />
            <circle cx="10.5" cy="10" r="1" fill="rgba(217,116,98,0.7)" />
          </svg>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  /* Full-viewport container so jumps + leftover spawns aren't clipped. */
  .cn-parade {
    position: fixed;
    inset: 0;
    pointer-events: none;
    z-index: var(--z-sticky, 50);
  }
  .cn-anim {
    position: absolute;
    bottom: 2px;
    left: 0;
    padding: 0;
    margin: 0;
    border: 0;
    background: transparent;
    pointer-events: auto;
    cursor: pointer;
    line-height: 0;
    outline: none;
  }
  .cn-anim:focus-visible {
    outline: 2px solid var(--acc);
    outline-offset: 2px;
    border-radius: 4px;
  }
  /* Direction-aware walk. Right-walkers flip the inner <g>. */
  .cn-anim[data-dir="right"] { animation: cn-walk-right var(--cn-dur) linear var(--cn-delay) infinite; }
  .cn-anim[data-dir="left"]  { animation: cn-walk-left  var(--cn-dur) linear var(--cn-delay) infinite; }
  .cn-anim[data-dir="right"] .cn-flip { transform: scaleX(-1); transform-origin: 50% 50%; }
  /* During interact: walk freezes (animation-play-state on the wrapper),
     and the inner SVG plays the kind-specific interact keyframe. */
  .cn-anim.interacting { animation-play-state: paused; }
  @keyframes cn-walk-right {
    0%   { transform: translateX(-40px); }
    100% { transform: translateX(calc(100vw + 40px)); }
  }
  @keyframes cn-walk-left {
    0%   { transform: translateX(calc(100vw + 40px)); }
    100% { transform: translateX(-40px); }
  }

  /* Walking gait cross-fade (cat/dog/bird use it; bunny opts out). */
  .cn-legs { opacity: 0; }
  .cn-legs-a { animation: cn-frame-a var(--cn-step) steps(1) infinite; }
  .cn-legs-b { animation: cn-frame-b var(--cn-step) steps(1) infinite; }
  @keyframes cn-frame-a { 0%, 49% { opacity: 1; } 50%, 100% { opacity: 0; } }
  @keyframes cn-frame-b { 0%, 49% { opacity: 0; } 50%, 100% { opacity: 1; } }

  /* Bunny's continuous hop. */
  .cn-svg-bunny {
    animation: cn-hop var(--cn-step) ease-in-out infinite;
    will-change: transform;
  }
  @keyframes cn-hop {
    0%, 100% { transform: translateY(0); }
    50%      { transform: translateY(-5px); }
  }

  /* Click jump — 600ms one-shot, composes on top of the bunny hop. */
  .cn-anim.jumping .cn-svg {
    animation: cn-jump 0.6s ease-out !important;
  }
  @keyframes cn-jump {
    0%   { transform: translateY(0)    scale(1);    }
    35%  { transform: translateY(-18px) scale(1.08); }
    70%  { transform: translateY(-3px)  scale(0.98); }
    100% { transform: translateY(0)    scale(1);    }
  }

  /* ─── INTERACT animations (per kind) ─────────────────────────────────
     Each kind has a distinct body motion + (where applicable) a part-level
     animation (cat paw, dog tail, bird beak). Animations run for ~1.2s
     while the wrapper walk is paused. */

  /* Cat: tilts forward and rocks while the front paw swipes 3 times. */
  .cn-anim-cat.interacting .cn-svg {
    animation: cn-cat-body 1.2s ease-in-out;
    transform-origin: 50% 100%;
  }
  @keyframes cn-cat-body {
    0%, 100% { transform: rotate(0)     translateY(0); }
    20%      { transform: rotate(-6deg) translateY(0); }
    40%      { transform: rotate(-4deg) translateY(-1px); }
    60%      { transform: rotate(-6deg) translateY(0); }
    80%      { transform: rotate(-3deg) translateY(-1px); }
  }
  .cn-paw { opacity: 0; }
  .cn-anim-cat.interacting .cn-paw {
    animation: cn-cat-paw 0.4s ease-in-out 3;
    transform-origin: 0% 50%;
  }
  @keyframes cn-cat-paw {
    0%   { opacity: 0; transform: translate(0, 0) rotate(0); }
    20%  { opacity: 1; transform: translate(-4px, -2px) rotate(-20deg); }
    60%  { opacity: 1; transform: translate(-6px, 1px)  rotate(15deg); }
    100% { opacity: 0; transform: translate(0, 0)       rotate(0); }
  }

  /* Dog: crouches (body lowers), tail wags double-time. */
  .cn-anim-dog.interacting .cn-svg {
    animation: cn-dog-crouch 1.2s ease-in-out;
    transform-origin: 50% 100%;
  }
  @keyframes cn-dog-crouch {
    0%, 100% { transform: translateY(0) scaleY(1);   }
    50%      { transform: translateY(2px) scaleY(0.88); }
  }
  .cn-anim-dog.interacting .cn-tail {
    animation: cn-dog-tail 0.18s ease-in-out infinite;
    transform-origin: 50% 100%;
  }
  @keyframes cn-dog-tail {
    0%, 100% { transform: rotate(-20deg); }
    50%      { transform: rotate(20deg); }
  }

  /* Bunny: squats down + ears wiggle (overrides hop during interact). */
  .cn-anim-bunny.interacting .cn-svg {
    animation: cn-bunny-nibble 1.2s ease-in-out !important;
    transform-origin: 50% 100%;
  }
  @keyframes cn-bunny-nibble {
    0%, 100% { transform: translateY(0) scaleY(1); }
    25%      { transform: translateY(3px) scaleY(0.92); }
    50%      { transform: translateY(2px) scaleY(0.94); }
    75%      { transform: translateY(3px) scaleY(0.92); }
  }

  /* Bird: head plunges down for rapid pecks via the beak group. */
  .cn-anim-bird.interacting .cn-svg {
    animation: cn-bird-bob 0.3s ease-in-out 4;
    transform-origin: 50% 100%;
  }
  @keyframes cn-bird-bob {
    0%, 100% { transform: translateY(0)  rotate(0); }
    50%      { transform: translateY(2px) rotate(-15deg); }
  }
  .cn-anim-bird.interacting .cn-beak {
    animation: cn-bird-peck 0.3s ease-in-out 4;
    transform-origin: 2px 10px;
  }
  @keyframes cn-bird-peck {
    0%, 100% { transform: translate(0, 0)    rotate(0); }
    50%      { transform: translate(-2px, 2px) rotate(-25deg); }
  }

  /* ─── Leftovers ──────────────────────────────────────────────────────
     Fixed-positioned at the captured screen coords; fade + drift down
     over LEFTOVER_TTL_MS. Centered via translate(-50%, -100%) so the
     spawn point reads as the BOTTOM of the leftover (the ground line). */
  .cn-leftover {
    position: fixed;
    transform: translate(-50%, -100%);
    pointer-events: none;
    animation: cn-leftover-life 4.5s ease-out forwards;
    will-change: opacity, transform;
  }
  @keyframes cn-leftover-life {
    0%   { opacity: 0; transform: translate(-50%, -100%) scale(0.55); }
    15%  { opacity: 1; transform: translate(-50%, -100%) scale(1);    }
    70%  { opacity: 1; transform: translate(-50%, -100%) scale(1);    }
    100% { opacity: 0; transform: translate(-50%, -85%)  scale(0.95); }
  }

  @media (prefers-reduced-motion: reduce) {
    .cn-parade { display: none; }
  }
</style>
