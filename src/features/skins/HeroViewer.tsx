import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { call } from "../../lib/api";

type Props = { hero: string; codes: string[]; addon: string | null };

// decoded models are heavy: only the last few are kept so switching back and forth is quick
const cache = new Map<string, ArrayBuffer>();
const KEEP = 4;

async function fetchModel(hero: string, codes: string[], addon: string | null): Promise<ArrayBuffer> {
  const key = `${hero}|${codes[0]}|${addon ?? ""}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const raw = await call<ArrayBuffer | number[]>("hero_model", { hero, codes, addon });
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

type Effect = { flags?: Record<string, number>; floats?: Record<string, number>; vectors?: Record<string, number[]> };
type Extras = { source2Effect?: Effect; outline?: { tint?: number[] | null } };

const lin = (v: number[] | undefined, k = 1) => new THREE.Color().setRGB((v?.[0] ?? 1) * k, (v?.[1] ?? 1) * k, (v?.[2] ?? 1) * k, THREE.LinearSRGBColorSpace);

/** The game's solid-colour outline, drawn as a slightly inflated back-face copy of the mesh. */
function outlines(model: THREE.Object3D, height: number): THREE.Mesh[] {
  const made: THREE.Mesh[] = [];
  const thick = height * 0.002;
  model.traverse((o) => {
    const m = o as THREE.Mesh;
    if (!m.isMesh || Array.isArray(m.material)) return;
    const ex = m.material.userData as Extras;
    const fx = ex.source2Effect;
    const tint = ex.outline?.tint;
    if (!fx?.flags?.F_SOLID_COLOR_OUTLINE || !tint) return;
    const lit = !!fx.flags.F_SELF_ILLUM;
    const col = lit ? fx.vectors?.g_vSelfIllumTint1 ?? tint : tint;
    const peak = Math.max(...col.slice(0, 3));
    if (peak < 0.25 && !lit) return; // the game's dark outlines add nothing here
    const k = peak > 0 ? 1.6 / peak : 1;
    const mat = new THREE.MeshBasicMaterial({ color: lin(col, lit || peak > 0.6 ? k * 0.7 : 1), side: THREE.BackSide, toneMapped: false });
    mat.onBeforeCompile = (sh) => {
      sh.vertexShader = sh.vertexShader.replace("#include <skinning_vertex>", `#include <skinning_vertex>
#ifdef USE_SKINNING
transformed += normalize(objectNormal) * ${thick.toFixed(5)};
#else
transformed += normalize(normal) * ${thick.toFixed(5)};
#endif`);
    };
    const skinned = m as THREE.SkinnedMesh;
    let copy: THREE.Mesh;
    if (skinned.isSkinnedMesh) {
      const sk = new THREE.SkinnedMesh(m.geometry, mat);
      sk.bind(skinned.skeleton, skinned.bindMatrix);
      copy = sk;
    } else {
      copy = new THREE.Mesh(m.geometry, mat);
    }
    copy.position.copy(m.position);
    copy.quaternion.copy(m.quaternion);
    copy.scale.copy(m.scale);
    copy.frustumCulled = false;
    copy.visible = m.visible;
    copy.userData.of = m;
    made.push(copy);
    m.parent?.add(copy);
  });
  return made;
}

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
  const { t } = useTranslation();
  const host = useRef<HTMLDivElement>(null);
  const api = useRef<{ show: (buf: ArrayBuffer) => Promise<BodyGroup[]>; reset: () => void; choose: (chosen: Record<string, number>) => void } | null>(null);
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
    scene.add(new THREE.HemisphereLight(0xffffff, 0x8a8aa0, 0.35));
    const key = new THREE.DirectionalLight(0xffffff, 1.1);
    key.position.set(2, 3, 4);
    scene.add(key);
    const rim = new THREE.DirectionalLight(0xdfe6ff, 0.6);
    rim.position.set(-3, 2, -3);
    scene.add(rim);
    // glow for self-lit parts: only values above the lit range bloom, so ordinary skins are unchanged
    const composer = new EffectComposer(renderer);
    composer.addPass(new RenderPass(scene, camera));
    const bloom = new UnrealBloomPass(new THREE.Vector2(1, 1), 0.35, 0.5, 1.0);
    composer.addPass(bloom);
    composer.addPass(new OutputPass());
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

    let raf = 0;
    const loop = () => {
      raf = requestAnimationFrame(loop);
      controls.update();
      mixer?.update(Math.min(clock.getDelta(), 0.1));
      for (const r of rims) r.visible = (r.userData.of as THREE.Mesh).visible;
      composer.render();
    };
    loop();

    const reset = () => {
      camera.position.copy(home.pos);
      controls.target.copy(home.target);
      controls.update();
    };
    api.current = {
      reset,
      show: async (buf) => {
        const gltf = await new GLTFLoader().parseAsync(buf.slice(0), "");
        if (model) {
          scene.remove(model);
          disposeTree(model);
        }
        mixer?.stopAllAction();
        mixer = null;
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
        scene.add(model);
        rims = [];
        const read = readGroups(model);
        table.current = read.groups;
        applyGroups(model, read.groups, read.chosen);
        // the idle animation of the hero, when the model carries one
        if (gltf.animations.length > 0) {
          mixer = new THREE.AnimationMixer(model);
          mixer.clipAction(gltf.animations[0]).play();
          clock.getDelta();
        }
        setChosen(read.chosen);
        const box = new THREE.Box3().setFromObject(model);
        const size = box.getSize(new THREE.Vector3());
        const mid = box.getCenter(new THREE.Vector3());
        const r = Math.max(size.x, size.y, size.z) || 1;
        rims = outlines(model, size.y || r);
        const dist = (r / 2 / Math.tan(THREE.MathUtils.degToRad(camera.fov / 2))) * 1.25;
        // the game's models face +X: look at the hero from the front, slightly to one side and above
        const dir = new THREE.Vector3(0.88, 0.12, 0.46).normalize();
        home = { pos: mid.clone().addScaledVector(dir, dist), target: mid };
        camera.near = r / 100;
        camera.far = r * 100;
        camera.updateProjectionMatrix();
        controls.minDistance = r * 0.3;
        controls.maxDistance = r * 6;
        reset();
        return read.groups;
      },
      choose: (c) => model && applyGroups(model, table.current, c),
    };
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
    setState("loading");
    fetchModel(hero, codes, addon)
      .then((buf) => api.current?.show(buf))
      .then((g) => {
        if (stale) return;
        setGroups(g ?? []);
        setState("ready");
      })
      .catch((e) => {
        if (stale) return;
        { const raw = e instanceof Error ? e.message : String(e); setError(t(`skins.err.${raw.split(":")[0]}`, { defaultValue: raw })); }
        setState("error");
      });
    return () => {
      stale = true;
    };
  }, [hero, addon, ready]);

  // Switches the game uses internally (ammo counts, reload poses, UI overlays) mean nothing to a person
  // looking at a skin; keep the few groups that change how the hero looks.
  const technical = /ammo|reload|clip|weapon|ui_|_ui|stake|drum|crossbow|toggle|quiver|sewer|banner|gun|default|hide|debug|fx|vfx|lod/i;
  const shown = groups.filter((g) => g.choices.length <= 6 && !technical.test(g.name) && !g.choices.some((c) => technical.test(c.name)));

  return (
    <div className="skin-viewer">
      <div ref={host} className="skin-canvas" />
      {state === "loading" && <div className="skin-veil">{t("skins.loading")}</div>}
      {state === "error" && (
        <div className="skin-veil skin-veil-error">
          <b>{t("skins.previewFailed")}</b>
          <span>{error}</span>
        </div>
      )}
      {shown.length > 0 && state === "ready" && showVariants && (
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
      <div className="skin-viewer-bar">
        <span>{t("skins.controls")}</span>
        {shown.length > 0 && (
          <button className="btn btn-sm" aria-pressed={showVariants} onClick={() => setShowVariants((v) => !v)}>
            {t("skins.variants")}
          </button>
        )}

        <button className="btn btn-sm" onClick={() => api.current?.reset()}>
          {t("skins.resetView")}
        </button>
      </div>
    </div>
  );
}
