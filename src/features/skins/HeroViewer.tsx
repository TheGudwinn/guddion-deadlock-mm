import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { SMAAPass } from "three/examples/jsm/postprocessing/SMAAPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { call } from "../../lib/api";
import Icon from "../../components/Icons";
import Select from "../../components/Select";
import { BG_COLOR, DEFAULT_CFG, LIGHTS, loadCfg, saveCapture, storeCfg, type Bg, type LightPreset, type ViewerCfg } from "./viewerConfig";

type Props = { hero: string; codes: string[]; addon: string | null };

// decoded models are heavy: only the last few are kept so switching back and forth is quick
const cache = new Map<string, ArrayBuffer>();
const KEEP = 4;

async function fetchModel(hero: string, codes: string[], addon: string | null, clip: string | null): Promise<ArrayBuffer> {
  const key = `${hero}|${codes[0]}|${addon ?? ""}|${clip ?? ""}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const raw = await call<ArrayBuffer | number[]>("hero_model", { hero, codes, addon, clip });
  const buf = raw instanceof ArrayBuffer ? raw : new Uint8Array(raw).buffer;
  cache.set(key, buf);
  while (cache.size > KEEP) cache.delete(cache.keys().next().value as string);
  return buf;
}

function disposeTree(root: THREE.Object3D) {
  root.traverse((o) => {
    const m = o as THREE.Mesh;
    if (!m.isMesh) return;
    m.geometry.dispose();
    for (const mat of Array.isArray(m.material) ? m.material : [m.material]) {
      for (const v of Object.values(mat)) if (v instanceof THREE.Texture) v.dispose();
      mat.dispose();
    }
  });
}

type ShowResult = { groups: BodyGroup[]; clips: string[]; clip: number };
type ViewerApi = {
  show: (buf: ArrayBuffer, keepView?: boolean) => Promise<ShowResult>;
  reset: () => void;
  choose: (chosen: Record<string, number>) => void;
  configure: (c: ViewerCfg) => void;
  snap: (x: number, y: number) => void;
  shot: () => Promise<Blob | null>;
  stream: () => MediaStream;
  play: (i: number) => void;
  setPlaying: (on: boolean) => void;
  seek: (t: number) => void;
  info: () => { time: number; dur: number };
};

const AXES = [
  { v: [1, 0, 0], color: "#e0674f", label: "X" },
  { v: [0, 1, 0], color: "#e6c35c", label: "Y" },
  { v: [0, 0, 1], color: "#6c9fd8", label: "Z" },
] as const;
const GIZMO = 96;

type Ability = { class: string; name: string; icon: string | null; clip: string | null };

const RU: Record<string, string> = {
  idle: "покой", melee: "ближний бой", dash: "рывок", parry: "парирование", reload: "перезарядка", shoot: "стрельба", cast: "каст",
  generic: "общий", ability: "способность", start: "начало", loop: "цикл", end: "конец", fire: "выстрел", release: "отпускание",
  hold: "удержание", draw: "натяжение", intro: "начало", outro: "конец", recover: "восстановление", channeling: "накачка",
  jump: "прыжок", slide: "скольжение", crouch: "приседание", stand: "стойка", hit: "удар", quick: "быстрый", air: "в воздухе",
  inair: "в воздухе", ground: "на земле", throw: "бросок", lift: "подъём", grab: "захват", knockdown: "нокдаун", large: "большой",
  medium: "средний", small: "малый", getup: "подъём", ui: "меню", hero: "герой", reveal: "появление", select: "выбор", pose: "поза",
  emote: "эмоция", sleep: "сон", shopping: "магазин", landing: "приземление", impact: "удар", item: "предмет", weapon: "оружие",
  charged: "заряженный", shot: "выстрел", guided: "управляемая", arrow: "стрела", power: "мощный", missile: "ракета", channel: "канал",
  run: "бег", stop: "остановка", turn: "поворот", left: "влево", right: "вправо", mantle: "подтягивание", death: "смерть",
  dive: "пикирование", hover: "парение", leap: "прыжок", precast: "подготовка", toss: "бросок", charge: "рывок",
};
/** "ability_charged_shot" as a readable label; Russian words where the dictionary has them. */
const clipLabel = (n: string, ru: boolean) =>
  n
    .split("_")
    .map((w) => (ru ? (RU[w] ?? w) : w))
    .join(" ");

const clock2 = (s: number) => Math.floor(s / 60) + ":" + (s % 60).toFixed(2).padStart(5, "0");

type Choice = { name: string; bit: number };
type BodyGroup = { name: string; choices: Choice[] };

/**
 * Switchable body groups of the model (a skin can ship a second form, such as a transformed ultimate).
 * The game shows one choice per group; a viewer that shows every mesh draws all forms at once.
 * Choices without meshes are left out: skins often keep the table but drop the alternates.
 */
function readGroups(root: THREE.Object3D): { groups: BodyGroup[]; chosen: Record<string, number> } {
  const meta = root.userData as { meshGroups?: { bodyGroup: string; name: string; bit: number }[]; defaultMeshGroups?: number[] };
  const drawn = new Set<number>();
  root.traverse((o) => {
    const bits = (o as THREE.Mesh).isMesh ? ((o as THREE.Mesh).geometry.userData as { meshGroups?: number[] }).meshGroups : undefined;
    for (const b of bits ?? []) drawn.add(b);
  });
  const byName = new Map<string, BodyGroup>();
  for (const c of meta.meshGroups ?? []) {
    if (!drawn.has(c.bit)) continue;
    const g = byName.get(c.bodyGroup) ?? { name: c.bodyGroup, choices: [] };
    g.choices.push({ name: c.name, bit: c.bit });
    byName.set(c.bodyGroup, g);
  }
  const shown = new Set(meta.defaultMeshGroups ?? []);
  const groups = [...byName.values()].filter((g) => g.choices.length > 1);
  const chosen: Record<string, number> = {};
  for (const g of groups) chosen[g.name] = (g.choices.find((c) => shown.has(c.bit)) ?? g.choices[0]).bit;
  return { groups, chosen };
}

/** Shows the meshes of the chosen choices and hides the other choices of the same groups. */
function applyGroups(root: THREE.Object3D, groups: BodyGroup[], chosen: Record<string, number>) {
  const switchable = new Set(groups.flatMap((g) => g.choices.map((c) => c.bit)));
  const shown = new Set(Object.values(chosen));
  root.traverse((o) => {
    const m = o as THREE.Mesh;
    if (!m.isMesh) return;
    const bits = (m.geometry.userData as { meshGroups?: number[] }).meshGroups;
    if (!bits || !bits.some((b) => switchable.has(b))) return;
    m.visible = bits.some((b) => shown.has(b));
  });
}

/** A model viewer: drag to orbit, wheel to zoom, right button to pan. */
export default function HeroViewer({ hero, codes, addon }: Props) {
  const { t, i18n } = useTranslation();
  const ru = i18n.language.startsWith("ru");
  const host = useRef<HTMLDivElement>(null);
  const api = useRef<ViewerApi | null>(null);
  const view = useRef<HTMLDivElement>(null);
  const gizmo = useRef<HTMLCanvasElement>(null);
  const [cfg, setCfg] = useState<ViewerCfg>(loadCfg);
  const cfgRef = useRef(cfg);
  cfgRef.current = cfg;
  const [menu, setMenu] = useState<"view" | "studio" | null>(null);
  const [clips, setClips] = useState<string[]>([]);
  const [playing, setPlaying] = useState(true);
  const [pos, setPos] = useState({ time: 0, dur: 0 });
  const [full, setFull] = useState(false);
  const [rec, setRec] = useState<{ at: number } | null>(null);
  const [secs, setSecs] = useState(0);
  const recorder = useRef<MediaRecorder | null>(null);
  const [note, setNote] = useState("");
  // the game's own animations of this hero (file stems); `clipName` is the one playing, null for the idle pose
  const [names, setNames] = useState<string[]>([]);
  const [abilities, setAbilities] = useState<Ability[]>([]);
  const [clipName, setClipName] = useState<string | null>(null);
  const [soft, setSoft] = useState(false);
  const loaded = useRef<{ hero: string; addon: string | null } | null>(null);
  const [groups, setGroups] = useState<BodyGroup[]>([]);
  const [chosen, setChosen] = useState<Record<string, number>>({});
  const table = useRef<BodyGroup[]>([]);
  const [showVariants, setShowVariants] = useState(false);
  const [state, setState] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    el.appendChild(renderer.domElement);
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x09090b);
    const camera = new THREE.PerspectiveCamera(35, 1, 0.01, 1000);
    // Image-based light from a neutral studio: surfaces keep the colours of their textures
    // (plain coloured lamps tint pale skin) and glossy parts have something to reflect.
    const pmrem = new THREE.PMREMGenerator(renderer);
    const room = new RoomEnvironment();
    scene.environment = pmrem.fromScene(room, 0.04).texture;
    scene.environmentIntensity = 1;
    const hemi = new THREE.HemisphereLight(0xffffff, 0x8a8aa0, 0.35);
    scene.add(hemi);
    const key = new THREE.DirectionalLight(0xffffff, 1.1);
    key.position.set(2, 3, 4);
    scene.add(key);
    const rim = new THREE.DirectionalLight(0xdfe6ff, 0.6);
    rim.position.set(-3, 2, -3);
    scene.add(rim);
    // glow only for strongly self-lit parts (threshold well above anything a lit surface reaches), so bright clothes never get a halo
    const composer = new EffectComposer(renderer);
    composer.addPass(new RenderPass(scene, camera));
    const bloom = new UnrealBloomPass(new THREE.Vector2(1, 1), 0.25, 0.3, 2.5);
    composer.addPass(bloom);
    composer.addPass(new OutputPass());
    // the composer's targets have no multisampling of their own, so edges are smoothed as a last pass
    const smaa = new SMAAPass(1, 1);
    composer.addPass(smaa);
    let grid: THREE.GridHelper | null = null;
    let clipList: THREE.AnimationClip[] = [];
    let action: THREE.AnimationAction | null = null;
    let live: ViewerCfg = cfgRef.current;
    let hits: { x: number; y: number; dir: THREE.Vector3 }[] = [];
    let rims: THREE.Mesh[] = [];
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    let model: THREE.Object3D | null = null;
    let mixer: THREE.AnimationMixer | null = null;
    const clock = new THREE.Clock();
    let home = { pos: new THREE.Vector3(0, 1, 4), target: new THREE.Vector3(0, 1, 0) };

    const resize = () => {
      const w = el.clientWidth || 1;
      const h = el.clientHeight || 1;
      renderer.setSize(w, h);
      composer.setPixelRatio(renderer.getPixelRatio());
      composer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    };
    const ro = new ResizeObserver(resize);
    ro.observe(el);
    resize();

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const drawGizmo = () => {
      const cv = gizmo.current;
      const ctx = cv?.getContext("2d");
      if (!cv || !ctx) return;
      if (cv.width !== GIZMO * dpr) {
        cv.width = GIZMO * dpr;
        cv.height = GIZMO * dpr;
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, GIZMO, GIZMO);
      const inv = camera.quaternion.clone().invert();
      const c = GIZMO / 2;
      const items = AXES.flatMap((a) =>
        [1, -1].map((sign) => {
          const dir = new THREE.Vector3(...a.v).multiplyScalar(sign);
          const p = dir.clone().applyQuaternion(inv);
          return { a, sign, dir, x: c + p.x * 30, y: c - p.y * 30, z: p.z };
        }),
      );
      items.sort((l, r) => l.z - r.z);
      hits = items.map((i) => ({ x: i.x, y: i.y, dir: i.dir }));
      ctx.font = "600 10px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      for (const i of items) {
        if (i.sign > 0) {
          ctx.strokeStyle = i.a.color;
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.moveTo(c, c);
          ctx.lineTo(i.x, i.y);
          ctx.stroke();
          ctx.fillStyle = i.a.color;
          ctx.beginPath();
          ctx.arc(i.x, i.y, 9, 0, Math.PI * 2);
          ctx.fill();
          ctx.fillStyle = "#14161c";
          ctx.fillText(i.a.label, i.x, i.y + 0.5);
        } else {
          ctx.fillStyle = i.a.color;
          ctx.globalAlpha = 0.45;
          ctx.beginPath();
          ctx.arc(i.x, i.y, 5, 0, Math.PI * 2);
          ctx.fill();
          ctx.globalAlpha = 1;
        }
      }
    };

    let raf = 0;
    const loop = () => {
      raf = requestAnimationFrame(loop);
      controls.update();
      mixer?.update(Math.min(clock.getDelta(), 0.1));
      for (const r of rims) r.visible = (r.userData.of as THREE.Mesh).visible;
      composer.render();
      if (live.gizmo) drawGizmo();
    };

    const configure = (c: ViewerCfg) => {
      live = c;
      const [env, k, r, f] = LIGHTS[c.light];
      scene.environmentIntensity = env * c.exposure;
      key.intensity = k * c.exposure;
      rim.intensity = r * c.exposure;
      hemi.intensity = f * c.exposure;
      bloom.enabled = c.glow;
      smaa.enabled = c.smaa;
      (scene.background as THREE.Color).set(BG_COLOR[c.bg]);
      if (camera.fov !== c.fov) {
        camera.fov = c.fov;
        camera.updateProjectionMatrix();
      }
      controls.autoRotate = c.spin;
      controls.autoRotateSpeed = 2.2;
      if (grid) grid.visible = c.floor;
      model?.traverse((o) => {
        const m = o as THREE.Mesh;
        if (!m.isMesh) return;
        for (const mat of Array.isArray(m.material) ? m.material : [m.material]) (mat as THREE.MeshStandardMaterial).wireframe = c.wire;
      });
    };

    const reset = () => {
      camera.position.copy(home.pos);
      controls.target.copy(home.target);
      controls.update();
    };
    api.current = {
      reset,
      configure,
      snap: (x, y) => {
        let best: (typeof hits)[number] | null = null;
        let d = 14;
        for (const h of hits) {
          const dd = Math.hypot(h.x - x, h.y - y);
          if (dd <= d) {
            d = dd;
            best = h;
          }
        }
        if (!best) return;
        const dist = camera.position.distanceTo(controls.target);
        // straight up or down needs a nudge, or the orbit has no side to lean on
        const dir = best.dir.clone().add(new THREE.Vector3(0.0001, 0, 0.0001)).normalize();
        camera.position.copy(controls.target).addScaledVector(dir, dist);
        controls.update();
      },
      shot: () => {
        composer.render();
        return new Promise((res) => renderer.domElement.toBlob(res, "image/png"));
      },
      stream: () => renderer.domElement.captureStream(30),
      play: (i) => {
        if (!mixer || !clipList[i]) return;
        action?.stop();
        action = mixer.clipAction(clipList[i]);
        action.reset().setLoop(THREE.LoopRepeat, Infinity).play();
        clock.getDelta();
      },
      setPlaying: (on) => {
        if (mixer) mixer.timeScale = on ? 1 : 0;
      },
      seek: (t) => {
        if (!mixer || !action) return;
        action.time = Math.max(0, Math.min(t, action.getClip().duration));
        const ts = mixer.timeScale;
        mixer.timeScale = 1;
        mixer.update(0);
        mixer.timeScale = ts;
      },
      info: () => ({ time: action?.time ?? 0, dur: action?.getClip().duration ?? 0 }),
      show: async (buf, keepView) => {
        const gltf = await new GLTFLoader().parseAsync(buf.slice(0), "");
        if (model) {
          scene.remove(model);
          disposeTree(model);
        }
        mixer?.stopAllAction();
        mixer = null;
        action = null;
        if (grid) {
          scene.remove(grid);
          grid.geometry.dispose();
          (grid.material as THREE.Material).dispose();
          grid = null;
        }
        model = gltf.scene;
        // double-sided: the game's models rely on shaders for what a plain material cannot draw
        model.traverse((o) => {
          const m = o as THREE.Mesh;
          if (!m.isMesh) return;
          for (const mat of Array.isArray(m.material) ? m.material : [m.material]) {
            mat.side = THREE.DoubleSide;
            // Mods often ship no metal map, and the fallback reads as fully metallic: without the game's own
            // shaders that draws the model black. Characters are matte; keep the roughness map only.
            const std = mat as THREE.MeshStandardMaterial;
            if (std.isMeshStandardMaterial) {
              std.metalnessMap = null;
              std.metalness = 0;
            }
          }
        });
        // Effect cards (flying paper, debris, shields) are cut out and animated by the game's own shaders
        // (jittered vertices, scrolling translucency masks). A plain material draws them as solid slabs that
        // cover the hero, so they are left out of the preview.
        const cards: THREE.Object3D[] = [];
        model.traverse((o) => {
          const m = o as THREE.Mesh;
          if (!m.isMesh || Array.isArray(m.material)) return;
          const mat = m.material as THREE.Material;
          const fx = (mat.userData as { source2Effect?: { flags?: Record<string, number> } }).source2Effect;
          if (fx?.flags?.F_JITTER_VERTICES && (mat.alphaTest > 0 || mat.transparent)) cards.push(m);
        });
        for (const c of cards) c.removeFromParent();
        scene.add(model);
        rims = [];
        const read = readGroups(model);
        table.current = read.groups;
        applyGroups(model, read.groups, read.chosen);
        // the idle animation of the hero is played first, when the model carries one
        clipList = gltf.animations;
        const idle = Math.max(0, clipList.findIndex((a) => /idle/i.test(a.name)));
        if (clipList.length > 0) {
          mixer = new THREE.AnimationMixer(model);
          action = mixer.clipAction(clipList[idle]);
          action.setLoop(THREE.LoopRepeat, Infinity).play();
          clock.getDelta();
        }
        setChosen(read.chosen);
        const box = new THREE.Box3().setFromObject(model);
        const size = box.getSize(new THREE.Vector3());
        const mid = box.getCenter(new THREE.Vector3());
        const r = Math.max(size.x, size.y, size.z) || 1;
        // No outline copies: the inflated back-face shells z-fight with the double-sided model and show up as
        // white/red speckles and stray spikes on many skins.
        rims = [];
        const dist = (r / 2 / Math.tan(THREE.MathUtils.degToRad(camera.fov / 2))) * 1.25;
        // the game's models face +X: look at the hero from the front, slightly to one side and above
        const dir = new THREE.Vector3(0.88, 0.12, 0.46).normalize();
        home = { pos: mid.clone().addScaledVector(dir, dist), target: mid };
        camera.near = r / 100;
        camera.far = r * 100;
        camera.updateProjectionMatrix();
        controls.minDistance = r * 0.3;
        controls.maxDistance = r * 6;
        if (!keepView) reset();
        grid = new THREE.GridHelper(r * 3, 24, 0x8a90a0, 0x8a90a0);
        grid.position.set(mid.x, box.min.y, mid.z);
        const gm = grid.material as THREE.LineBasicMaterial;
        gm.transparent = true;
        gm.opacity = 0.22;
        scene.add(grid);
        configure(live);
        return { groups: read.groups, clips: clipList.map((a, i) => a.name || "#" + (i + 1)), clip: idle };
      },
      choose: (c) => model && applyGroups(model, table.current, c),
    };
    configure(live);
    loop();
    setReady(true);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      controls.dispose();
      composer.dispose();
      if (model) disposeTree(model);
      scene.environment?.dispose();
      pmrem.dispose();
      room.dispose();
      renderer.dispose();
      renderer.domElement.remove();
      api.current = null;
    };
  }, []);

  useEffect(() => {
    if (!ready) return;
    let stale = false;
    // changing the animation of the model that is already shown keeps the camera and the veil away
    const same = loaded.current?.hero === hero && loaded.current.addon === addon;
    const clipNow = same ? clipName : null;
    if (!same && clipName !== null) setClipName(null);
    setSoft(same);
    setState("loading");
    fetchModel(hero, codes, addon, clipNow)
      .then((buf) => api.current?.show(buf, same))
      .then((res) => {
        if (stale) return;
        loaded.current = { hero, addon };
        setGroups(res?.groups ?? []);
        setClips(res?.clips ?? []);
        setPlaying(true);
        setSoft(false);
        setState("ready");
      })
      .catch((e) => {
        if (stale) return;
        { const raw = e instanceof Error ? e.message : String(e); console.error("hero model", raw); setError(t(`skins.err.${raw.split(":")[0]}`, { defaultValue: t("skins.err.generic") })); }
        setState("error");
      });
    return () => {
      stale = true;
    };
  }, [hero, addon, ready, clipName]);

  useEffect(() => {
    let stale = false;
    setNames([]);
    call<string[]>("hero_clips", { hero }).then(
      (r) => !stale && setNames(r ?? []),
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [hero]);

  // the four abilities of the hero: name in the app's language, icon, and the animation that shows each one
  useEffect(() => {
    let stale = false;
    setAbilities([]);
    call<Ability[]>("hero_abilities", { hero, lang: ru ? "russian" : "english" }).then(
      (r) => !stale && setAbilities(r ?? []),
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [hero, ru]);

  // Switches the game uses internally (ammo counts, reload poses, UI overlays) mean nothing to a person
  // looking at a skin; keep the few groups that change how the hero looks.
  const technical = /ammo|reload|clip|weapon|ui_|_ui|stake|drum|crossbow|toggle|quiver|sewer|banner|gun|default|hide|debug|fx|vfx|lod/i;
  const shown = groups.filter((g) => g.choices.length <= 6 && !technical.test(g.name) && !g.choices.some((c) => technical.test(c.name)));

  const up = (patch: Partial<ViewerCfg>) => setCfg((c) => ({ ...c, ...patch }));
  useEffect(() => {
    api.current?.configure(cfg);
    storeCfg(cfg);
  }, [cfg, ready]);

  // timeline position, read from the player a few times a second
  useEffect(() => {
    if (!ready || clips.length === 0) return;
    const id = window.setInterval(() => {
      const i = api.current?.info();
      if (i) setPos((p) => (Math.abs(p.time - i.time) < 0.01 && p.dur === i.dur ? p : i));
    }, 90);
    return () => window.clearInterval(id);
  }, [ready, clips.length]);

  useEffect(() => {
    const on = () => setFull(document.fullscreenElement === view.current);
    document.addEventListener("fullscreenchange", on);
    return () => document.removeEventListener("fullscreenchange", on);
  }, []);

  // closes an open menu on a click elsewhere or Escape
  useEffect(() => {
    if (!menu) return;
    const down = (e: PointerEvent) => {
      if (!(e.target as Element).closest(".vw-pop, .vw-menu-btn")) setMenu(null);
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && setMenu(null);
    document.addEventListener("pointerdown", down);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", down);
      document.removeEventListener("keydown", key);
    };
  }, [menu]);

  useEffect(() => {
    if (!rec) return;
    const id = window.setInterval(() => setSecs(Math.floor((Date.now() - rec.at) / 1000)), 250);
    return () => window.clearInterval(id);
  }, [rec]);
  useEffect(() => {
    if (!note) return;
    const id = window.setTimeout(() => setNote(""), 2600);
    return () => window.clearTimeout(id);
  }, [note]);
  useEffect(() => () => recorder.current?.stop(), []);

  const fileBase = hero + (addon ? "-" + addon.replace(/\.[^.]+$/, "") : "");

  const screenshot = async () => {
    const blob = await api.current?.shot();
    if (!blob) return;
    try {
      if (await saveCapture(blob, fileBase, "png", t("skins.vw.saveShot"))) setNote(t("skins.vw.saved"));
    } catch {
      setNote(t("skins.vw.saveFailed"));
    }
  };

  const toggleRecord = () => {
    if (recorder.current) {
      recorder.current.stop();
      return;
    }
    const a = api.current;
    if (!a || typeof MediaRecorder === "undefined") return setNote(t("skins.vw.noRecord"));
    const type = ["video/webm;codecs=vp9", "video/webm;codecs=vp8", "video/webm"].find((m) => MediaRecorder.isTypeSupported(m));
    if (!type) return setNote(t("skins.vw.noRecord"));
    const chunks: Blob[] = [];
    const r = new MediaRecorder(a.stream(), { mimeType: type, videoBitsPerSecond: 8_000_000 });
    r.ondataavailable = (e) => e.data.size > 0 && chunks.push(e.data);
    r.onstop = () => {
      recorder.current = null;
      setRec(null);
      saveCapture(new Blob(chunks, { type: "video/webm" }), fileBase, "webm", t("skins.vw.saveVideo")).then(
        (ok) => ok && setNote(t("skins.vw.saved")),
        () => setNote(t("skins.vw.saveFailed")),
      );
    };
    r.start(500);
    recorder.current = r;
    setSecs(0);
    setRec({ at: Date.now() });
  };

  const fullscreen = () => {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void view.current?.requestFullscreen().catch(() => {});
  };

  const pickClip = (v: string) => setClipName(v === "" ? null : v);
  const togglePlay = () => {
    const next = !playing;
    setPlaying(next);
    api.current?.setPlaying(next);
  };

  const ready2 = state === "ready";
  return (
    <div ref={view} className={"skin-viewer" + (full ? " is-full" : "")}>
      <div ref={host} className="skin-canvas" title={t("skins.controls")} />
      {state === "loading" && !soft && <div className="skin-veil">{t("skins.loading")}</div>}
      {state === "loading" && soft && <div className="vw-busy">{t("skins.vw.busy")}</div>}
      {state === "error" && (
        <div className="skin-veil skin-veil-error">
          <b>{t("skins.previewFailed")}</b>
          <span>{error}</span>
        </div>
      )}

      <div className="vw-top">
        <div className="vw-menus">
          <button className="vw-pill vw-menu-btn" aria-expanded={menu === "view"} onClick={() => setMenu(menu === "view" ? null : "view")}>
            <Icon name="sliders" size={15} />
            {t("skins.vw.view")}
          </button>
          <button className="vw-pill vw-menu-btn" aria-expanded={menu === "studio"} onClick={() => setMenu(menu === "studio" ? null : "studio")}>
            <Icon name="image" size={15} />
            {t("skins.vw.studio")}
          </button>
          <button className="vw-tag" aria-pressed={cfg.smaa} title={t("skins.vw.smaaHint")} onClick={() => up({ smaa: !cfg.smaa })}>
            SMAA
          </button>
        </div>
        {cfg.gizmo && (
          <canvas
            ref={gizmo}
            className="vw-gizmo"
            style={{ width: GIZMO, height: GIZMO }}
            title={t("skins.vw.gizmoHint")}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              api.current?.snap(e.clientX - r.left, e.clientY - r.top);
            }}
          />
        )}
      </div>

      {menu === "view" && (
        <div className="vw-pop" role="dialog" aria-label={t("skins.vw.view")}>
          <Row label={t("skins.vw.wire")} on={cfg.wire} onChange={(v) => up({ wire: v })} />
          <Row label={t("skins.vw.floor")} on={cfg.floor} onChange={(v) => up({ floor: v })} />
          <Row label={t("skins.vw.gizmo")} on={cfg.gizmo} onChange={(v) => up({ gizmo: v })} />
          <Row label={t("skins.vw.spin")} on={cfg.spin} onChange={(v) => up({ spin: v })} />
          {shown.length > 0 && ready2 && <Row label={t("skins.variants")} on={showVariants} onChange={setShowVariants} />}
          <Slide label={t("skins.vw.fov")} min={20} max={70} step={1} value={cfg.fov} fmt={(v) => v + "°"} onChange={(v) => up({ fov: v })} />
          <button className="btn btn-sm vw-reset" onClick={() => api.current?.reset()}>
            {t("skins.resetView")}
          </button>
        </div>
      )}
      {menu === "studio" && (
        <div className="vw-pop" role="dialog" aria-label={t("skins.vw.studio")}>
          <Pick label={t("skins.vw.bg")} value={cfg.bg} options={BGS.map((b) => ({ value: b, label: t("skins.vw.bg_" + b) }))} onChange={(v) => up({ bg: v as Bg })} />
          <Pick label={t("skins.vw.light")} value={cfg.light} options={LIGHT_KEYS.map((b) => ({ value: b, label: t("skins.vw.light_" + b) }))} onChange={(v) => up({ light: v as LightPreset })} />
          <Slide label={t("skins.vw.exposure")} min={0.5} max={1.8} step={0.05} value={cfg.exposure} fmt={(v) => v.toFixed(2)} onChange={(v) => up({ exposure: v })} />
          <Row label={t("skins.vw.glow")} on={cfg.glow} onChange={(v) => up({ glow: v })} />
          <button className="btn btn-sm vw-reset" onClick={() => setCfg(DEFAULT_CFG)}>
            {t("skins.vw.defaults")}
          </button>
        </div>
      )}

      <div className="vw-side">
        <button className="vw-ico" aria-label={t("skins.resetView")} title={t("skins.resetView")} onClick={() => api.current?.reset()}>
          <Icon name="frame" size={17} />
        </button>
        <button className="vw-ico" aria-pressed={cfg.spin} aria-label={t("skins.vw.spin")} title={t("skins.vw.spin")} onClick={() => up({ spin: !cfg.spin })}>
          <Icon name="spin" size={17} />
        </button>
        <button className="vw-ico" aria-label={t("skins.vw.shot")} title={t("skins.vw.shot")} disabled={!ready2} onClick={() => void screenshot()}>
          <Icon name="camera" size={17} />
        </button>
        <button className={"vw-ico" + (rec ? " is-rec" : "")} aria-pressed={!!rec} aria-label={rec ? t("skins.vw.stop") : t("skins.vw.record")} title={rec ? t("skins.vw.stop") : t("skins.vw.record")} disabled={!ready2} onClick={toggleRecord}>
          <Icon name="video" size={17} />
        </button>
        <button className="vw-ico" aria-pressed={full} aria-label={t("skins.vw.full")} title={t("skins.vw.full")} onClick={fullscreen}>
          <Icon name="expand" size={17} />
        </button>
      </div>

      {rec && (
        <div className="vw-rec" role="status">
          <i /> {clock2(secs)}
        </div>
      )}
      {note && (
        <div className="vw-note" role="status">
          {note}
        </div>
      )}

      {shown.length > 0 && ready2 && showVariants && (
        <div className="skin-variants">
          {shown.map((g) => (
            <div key={g.name} className="skin-variant-row">
              <span>{g.name}</span>
              {g.choices.map((c) => (
                <button
                  key={c.bit}
                  className="chip"
                  aria-pressed={chosen[g.name] === c.bit}
                  onClick={() => {
                    const next = { ...chosen, [g.name]: c.bit };
                    setChosen(next);
                    api.current?.choose(next);
                  }}
                >
                  {c.name}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}

      {abilities.length > 0 && (ready2 || soft) && (
        <div className="vw-slots" role="group" aria-label={t("skins.vw.abilities")}>
          {abilities.map((a, i) => (
            <button
              key={a.class}
              className="vw-slot"
              aria-pressed={a.clip !== null && clipName === a.clip}
              disabled={!a.clip}
              title={a.clip ? a.name : t("skins.vw.noClip", { name: a.name })}
              onClick={() => a.clip && pickClip(clipName === a.clip ? "" : a.clip)}
            >
              <span className="vw-slot-box">
                {a.icon ? <img src={a.icon} alt="" loading="lazy" draggable={false} /> : <b>{i + 1}</b>}
                <i className="vw-slot-n">{i + 1}</i>
              </span>
              <span className="vw-slot-name">{a.name}</span>
            </button>
          ))}
        </div>
      )}

      {(clips.length > 0 || names.length > 0) && (ready2 || soft) && (
        <div className="vw-anim">
          <button className="vw-play" aria-label={playing ? t("skins.vw.pause") : t("skins.vw.play")} onClick={togglePlay}>
            <Icon name={playing ? "pause" : "play"} size={16} />
          </button>
          <Select className="vw-clip" label={t("skins.vw.clip")} value={clipName ?? ""} onChange={pickClip} options={[{ value: "", label: t("skins.vw.idle") }, ...names.map((n) => ({ value: n, label: clipLabel(n, ru) }))]} />
          <span className="vw-time">{clock2(pos.time)}</span>
          <input
            className="vw-scrub"
            type="range"
            min={0}
            max={pos.dur || 1}
            step={0.01}
            value={Math.min(pos.time, pos.dur || 1)}
            aria-label={t("skins.vw.timeline")}
            onChange={(e) => {
              const v = Number(e.target.value);
              setPos((p) => ({ ...p, time: v }));
              api.current?.seek(v);
            }}
          />
          <span className="vw-time vw-dim">{clock2(pos.dur)}</span>
        </div>
      )}
    </div>
  );
}

const BGS: Bg[] = ["dark", "slate", "light"];
const LIGHT_KEYS: LightPreset[] = ["studio", "soft", "dramatic"];

function Row({ label, on, onChange }: { label: string; on: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="vw-row">
      <span>{label}</span>
      <input type="checkbox" role="switch" className="vw-switch" checked={on} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

function Slide({ label, min, max, step, value, fmt, onChange }: { label: string; min: number; max: number; step: number; value: number; fmt: (v: number) => string; onChange: (v: number) => void }) {
  return (
    <label className="vw-row vw-slide">
      <span>{label}</span>
      <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => onChange(Number(e.target.value))} />
      <b>{fmt(value)}</b>
    </label>
  );
}

function Pick({ label, value, options, onChange }: { label: string; value: string; options: { value: string; label: string }[]; onChange: (v: string) => void }) {
  return (
    <div className="vw-pick">
      <span>{label}</span>
      <div className="vw-segs" role="radiogroup" aria-label={label}>
        {options.map((o) => (
          <button key={o.value} role="radio" aria-checked={value === o.value} className="vw-seg" onClick={() => onChange(o.value)}>
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
