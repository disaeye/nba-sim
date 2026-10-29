// 2.5D 球场视图：正交平行投影下的标准球场地板、篮架、球员小人模型与投篮飞行。
//
// 数据只来自协议帧：位置、朝向、球的状态与每 tick 的事件名。
// 动画状态由此层推导，引擎不感知渲染。

import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { clone as cloneSkinned } from "three/addons/utils/SkeletonUtils.js";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";

const DEFAULT_COURT = {
  length_ft: 94,
  width_ft: 50,
  hoop_left_x_ft: 5.25,
  hoop_right_x_ft: 88.75,
  hoop_y_ft: 25,
  three_point_distance_ft: 23.75,
};

const LANE_WIDTH_FT = 16;
const LANE_LENGTH_FT = 19;
const FREE_THROW_CIRCLE_FT = 6;
const CENTER_CIRCLE_FT = 6;
const RESTRICTED_ARC_FT = 4;
const CORNER_THREE_FROM_SIDELINE_FT = 3;
const RIM_HEIGHT_FT = 10;
const RIM_RADIUS_FT = 0.75;
const BACKBOARD_WIDTH_FT = 6;
const BACKBOARD_HEIGHT_FT = 3.5;
const BACKBOARD_BOTTOM_FT = 9.5;
const BACKBOARD_FROM_BASELINE_FT = 4;
const BALL_RADIUS_FT = 0.4;
const BALL_TRAIL_POINTS = 28;
const PLAYER_HEIGHT_FT = 7.0;

const TEAM_COLORS = { home: 0x1fbf8f, away: 0xf2a13c };
const TEAM_DARK = { home: 0x0c6a4f, away: 0x9a5a12 };

const CLIP_NAMES = {
  idle: "Man_Idle",
  walk: "Man_Walk",
  run: "Man_Run",
  shot: "Man_Punch",
  pass: "Man_Punch",
  jump: "Man_Jump",
  celebrate: "Man_Clapping",
};

const GESTURES = {
  shot: { clip: CLIP_NAMES.shot, speed: 1.0 },
  pass: { clip: CLIP_NAMES.pass, speed: 1.35 },
  rebound: { clip: CLIP_NAMES.jump, speed: 1.0 },
  celebrate: { clip: CLIP_NAMES.celebrate, speed: 1.0 },
};

const SPEED_IDLE_FTPS = 1.2;
const SPEED_RUN_FTPS = 7.5;
const WALK_STRIDE_FT = 5.0;
const RUN_STRIDE_FT = 14.0;

const PRESETS = {
  broadcast: { azimuth: 0, elevation: 34, coverage: 104, target: [0, 1.2, 0] },
  follow: { azimuth: 0, elevation: 34, coverage: 62 },
  top: { azimuth: 0, elevation: 89.5, coverage: 104, target: [0, 0, 0] },
};

function readCourt(rules) {
  return {
    length: finite(rules?.court_width_ft, DEFAULT_COURT.length_ft),
    width: finite(rules?.court_height_ft, DEFAULT_COURT.width_ft),
    leftHoop: finite(rules?.hoop_left_x_ft, DEFAULT_COURT.hoop_left_x_ft),
    rightHoop: finite(rules?.hoop_right_x_ft, DEFAULT_COURT.hoop_right_x_ft),
    hoopY: finite(rules?.hoop_y_ft, DEFAULT_COURT.hoop_y_ft),
    threePoint: finite(
      rules?.three_point_distance_ft,
      DEFAULT_COURT.three_point_distance_ft,
    ),
  };
}

function finite(value, fallback) {
  return Number.isFinite(value) ? Number(value) : fallback;
}

function seededRandom(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function shortestAngle(from, to) {
  let delta = (to - from) % (Math.PI * 2);
  if (delta > Math.PI) delta -= Math.PI * 2;
  if (delta < -Math.PI) delta += Math.PI * 2;
  return delta;
}

class LineBuilder {
  constructor() {
    this.positions = [];
  }

  segment(fromX, fromZ, toX, toZ, y = 0) {
    this.segment3(fromX, y, fromZ, toX, y, toZ);
  }

  segment3(fromX, fromY, fromZ, toX, toY, toZ) {
    this.positions.push(fromX, fromY, fromZ, toX, toY, toZ);
  }

  polyline(points, y = 0) {
    for (let index = 0; index < points.length - 1; index += 1) {
      this.segment(points[index][0], points[index][1], points[index + 1][0], points[index + 1][1], y);
    }
  }

  arc(centerX, centerZ, radius, startAngle, endAngle, steps = 48, y = 0) {
    const points = [];
    for (let step = 0; step <= steps; step += 1) {
      const angle = startAngle + ((endAngle - startAngle) * step) / steps;
      points.push([centerX + Math.cos(angle) * radius, centerZ + Math.sin(angle) * radius]);
    }
    this.polyline(points, y);
  }

  circle(centerX, centerZ, radius, steps = 64, y = 0) {
    this.arc(centerX, centerZ, radius, 0, Math.PI * 2, steps, y);
  }

  toGeometry() {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.Float32BufferAttribute(this.positions, 3));
    return geometry;
  }
}

function makeWoodTexture() {
  const canvas = document.createElement("canvas");
  canvas.width = 1024;
  canvas.height = 512;
  const ctx = canvas.getContext("2d");
  const random = seededRandom(20261023);
  ctx.fillStyle = "#d9bf90";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  const plankWidth = 64;
  for (let x = 0; x < canvas.width; x += plankWidth) {
    const shade = 0.94 + random() * 0.09;
    ctx.fillStyle = `rgba(120, 84, 42, ${(1 - shade) * 0.6 + 0.03})`;
    ctx.fillRect(x, 0, plankWidth, canvas.height);
    ctx.strokeStyle = "rgba(92, 62, 30, 0.22)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x + 0.5, 0);
    ctx.lineTo(x + 0.5, canvas.height);
    ctx.stroke();
    for (let line = 0; line < 5; line += 1) {
      const offset = x + 6 + random() * (plankWidth - 12);
      ctx.strokeStyle = `rgba(126, 92, 52, ${0.05 + random() * 0.06})`;
      ctx.beginPath();
      ctx.moveTo(offset, 0);
      for (let y = 0; y <= canvas.height; y += 32) {
        ctx.lineTo(offset + Math.sin(y * 0.03 + line) * 2.4, y);
      }
      ctx.stroke();
    }
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.ClampToEdgeWrapping;
  texture.repeat.set(3, 1);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = 4;
  return texture;
}

function makeLogoTexture() {
  const canvas = document.createElement("canvas");
  canvas.width = 512;
  canvas.height = 512;
  const ctx = canvas.getContext("2d");
  ctx.clearRect(0, 0, 512, 512);
  ctx.strokeStyle = "rgba(60, 44, 24, 0.35)";
  ctx.lineWidth = 6;
  ctx.beginPath();
  ctx.arc(256, 256, 232, 0, Math.PI * 2);
  ctx.stroke();
  ctx.fillStyle = "rgba(58, 42, 22, 0.4)";
  ctx.font = "800 132px 'Plus Jakarta Sans', system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("NBA", 256, 208);
  ctx.font = "800 96px 'Plus Jakarta Sans', system-ui, sans-serif";
  ctx.fillText("SIM", 256, 316);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

function makeNumberTexture(jersey, colorHex) {
  const canvas = document.createElement("canvas");
  canvas.width = 128;
  canvas.height = 128;
  const ctx = canvas.getContext("2d");
  ctx.clearRect(0, 0, 128, 128);
  ctx.fillStyle = `#${colorHex.toString(16).padStart(6, "0")}`;
  ctx.beginPath();
  ctx.arc(64, 64, 58, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = "rgba(255, 255, 255, 0.85)";
  ctx.lineWidth = 6;
  ctx.beginPath();
  ctx.arc(64, 64, 55, 0, Math.PI * 2);
  ctx.stroke();
  ctx.fillStyle = "#ffffff";
  ctx.font = "800 64px 'Plus Jakarta Sans', system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(String(jersey ?? ""), 64, 68);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

let toonGradient = null;

// 三段式卡通渐层：把体积光拆成平坦色块，让模型读起来接近手绘小人。
function sharedToonGradient() {
  if (toonGradient) return toonGradient;
  const data = new Uint8Array([96, 168, 232, 255]);
  toonGradient = new THREE.DataTexture(data, data.length, 1, THREE.RedFormat);
  toonGradient.minFilter = THREE.NearestFilter;
  toonGradient.magFilter = THREE.NearestFilter;
  toonGradient.generateMipmaps = false;
  toonGradient.needsUpdate = true;
  return toonGradient;
}

function measureModel(template) {
  const box = new THREE.Box3();
  const vertex = new THREE.Vector3();
  template.traverse((object) => {
    if (!object.isSkinnedMesh && !object.isMesh) return;
    const position = object.geometry?.attributes?.position;
    if (!position) return;
    const step = Math.max(1, Math.floor(position.count / 1200));
    for (let index = 0; index < position.count; index += step) {
      if (object.isSkinnedMesh) object.getVertexPosition(index, vertex);
      else vertex.fromBufferAttribute(position, index);
      object.localToWorld(vertex);
      box.expandByPoint(vertex);
    }
  });
  const height = Math.max(0.001, box.max.y - box.min.y);
  const scale = PLAYER_HEIGHT_FT / height;
  return {
    scale,
    footOffset: -box.min.y * scale,
    forwardOffset: measureForwardOffset(template),
  };
}

// 脚尖小骨从脚跟在水平面上指向前方；它的方向定义模型的默认前向，用于对齐偏航。
function measureForwardOffset(template) {
  const pairs = [
    ["FootR", "FootR_end"],
    ["FootL", "FootL_end"],
    ["foot_r", "foot_r_end"],
  ];
  for (const [footName, toeName] of pairs) {
    const foot = template.getObjectByName(footName);
    const toe = template.getObjectByName(toeName);
    if (!foot || !toe) continue;
    const from = foot.getWorldPosition(new THREE.Vector3());
    const to = toe.getWorldPosition(new THREE.Vector3());
    const dx = to.x - from.x;
    const dz = to.z - from.z;
    if (Math.hypot(dx, dz) < 1e-4) continue;
    return -Math.atan2(dx, dz);
  }
  return 0;
}

function collectRestPose(root) {
  const rest = new Map();
  root.traverse((object) => {
    if (!object.isBone) return;
    rest.set(object.name, {
      position: object.position.toArray(),
      quaternion: object.quaternion.toArray(),
      scale: object.scale.toArray(),
    });
  });
  return rest;
}

function completeClip(clip, rest, cache) {
  if (cache.has(clip.name)) return cache.get(clip.name);
  const existing = new Set(clip.tracks.map((track) => track.name));
  const duration = clip.duration > 0 ? clip.duration : 1;
  const tracks = clip.tracks.slice();
  for (const [bone, pose] of rest) {
    const properties = [
      ["position", pose.position, THREE.VectorKeyframeTrack],
      ["quaternion", pose.quaternion, THREE.QuaternionKeyframeTrack],
      ["scale", pose.scale, THREE.VectorKeyframeTrack],
    ];
    for (const [property, values, TrackClass] of properties) {
      const name = `${bone}.${property}`;
      if (existing.has(name)) continue;
      const times = new Float32Array([0, duration]);
      const frames = new Float32Array(values.length * 2);
      frames.set(values, 0);
      frames.set(values, values.length);
      tracks.push(new TrackClass(name, times, frames));
    }
  }
  const completed = new THREE.AnimationClip(clip.name, clip.duration, tracks);
  cache.set(clip.name, completed);
  return completed;
}

class PlayerFigure {
  constructor({ template, clips, team, numberTexture, metrics }) {
    this.root = new THREE.Group();
    this.model = cloneSkinned(template);
    this.model.scale.setScalar(metrics.scale);
    this.model.position.y = metrics.footOffset;
    this.forwardOffset = metrics.forwardOffset;
    const primary = new THREE.Color(TEAM_COLORS[team] ?? TEAM_COLORS.home);
    const dark = new THREE.Color(TEAM_DARK[team] ?? TEAM_DARK.home);
    const materials = new Map();
    const gradient = sharedToonGradient();
    this.model.traverse((object) => {
      if (!object.isMesh && !object.isSkinnedMesh) return;
      object.castShadow = true;
      object.receiveShadow = false;
      object.frustumCulled = false;
      const material = object.material;
      if (!materials.has(material.uuid)) {
        const color = material.color.clone();
        if (material.name === "Shirt") color.copy(primary);
        else if (material.name === "Pants") color.copy(dark);
        materials.set(
          material.uuid,
          new THREE.MeshToonMaterial({ color, gradientMap: gradient }),
        );
      }
      object.material = materials.get(material.uuid);
    });
    this.root.add(this.model);

    this.ring = new THREE.Mesh(
      new THREE.RingGeometry(0.95, 1.25, 40),
      new THREE.MeshBasicMaterial({
        color: primary,
        transparent: true,
        opacity: 0.62,
        depthWrite: false,
        side: THREE.DoubleSide,
      }),
    );
    this.ring.rotation.x = -Math.PI / 2;
    this.ring.position.y = 0.035;
    this.ring.renderOrder = 1;
    this.root.add(this.ring);

    this.label = new THREE.Sprite(
      new THREE.SpriteMaterial({
        map: numberTexture,
        transparent: true,
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.label.scale.set(2.8, 2.8, 1);
    this.label.position.y = PLAYER_HEIGHT_FT + 1.4;
    this.label.renderOrder = 12;
    this.root.add(this.label);

    this.mixer = new THREE.AnimationMixer(this.model);
    this.actions = new Map();
    for (const clip of clips) {
      const action = this.mixer.clipAction(clip);
      action.enabled = true;
      this.actions.set(clip.name, action);
    }
    this.current = null;
    this.gesture = null;
    this.gestureUntil = 0;
    this.yaw = 0;
    this.targetYaw = 0;
    this.speed = 0;
    this.fist =
      this.model.getObjectByName("PalmR") ??
      this.model.getObjectByName("MiddleHandR") ??
      null;
  }

  resolveClip(suffix) {
    for (const name of this.actions.keys()) {
      if (name === suffix || name.endsWith(`|${suffix}`)) return this.actions.get(name);
    }
    return null;
  }

  fadeTo(action, duration = 0.16) {
    if (!action || action === this.current) return;
    action.reset();
    action.setEffectiveWeight(1);
    action.setEffectiveTimeScale(1);
    action.setLoop(THREE.LoopRepeat, Infinity);
    action.clampWhenFinished = false;
    action.play();
    if (this.current) this.current.crossFadeTo(action, duration, false);
    this.current = action;
  }

  playGesture(kind) {
    const spec = GESTURES[kind];
    if (!spec) return;
    const action = this.resolveClip(spec.clip);
    if (!action) return;
    action.reset();
    action.setLoop(THREE.LoopOnce, 1);
    action.clampWhenFinished = true;
    action.setEffectiveWeight(1);
    action.setEffectiveTimeScale(spec.speed);
    action.play();
    if (this.current && this.current !== action) this.current.crossFadeTo(action, 0.08, false);
    this.current = action;
    this.gesture = { action, kind };
    this.gestureUntil = performance.now() + (action.getClip().duration / spec.speed) * 1000;
  }

  applyLocomotion(now) {
    if (this.gesture && now < this.gestureUntil) return;
    this.gesture = null;
    const action =
      this.speed < SPEED_IDLE_FTPS
        ? this.resolveClip(CLIP_NAMES.idle)
        : this.speed < SPEED_RUN_FTPS
          ? this.resolveClip(CLIP_NAMES.walk)
          : this.resolveClip(CLIP_NAMES.run);
    if (!action) return;
    const stride = this.speed < SPEED_RUN_FTPS ? WALK_STRIDE_FT : RUN_STRIDE_FT;
    const scale = Math.min(1.75, Math.max(0.55, (this.speed / stride) * action.getClip().duration));
    this.fadeTo(action);
    if (this.current === action) action.setEffectiveTimeScale(scale);
  }

  update(delta, now) {
    this.applyLocomotion(now);
    this.mixer.update(delta);
    this.model.updateMatrixWorld(true);
  }

  place(x, z, yaw, delta) {
    this.root.position.set(x, 0, z);
    if (!this.placed) {
      this.placed = true;
      this.yaw = yaw;
    }
    this.targetYaw = yaw;
    const step = Math.min(1, Math.max(0, delta) * 14);
    this.yaw += shortestAngle(this.yaw, yaw) * step;
    this.model.rotation.y = this.yaw + this.forwardOffset;
  }
}

export class Court3D {
  constructor({ canvas, state, statusEl, overlayEl }) {
    this.canvas = canvas;
    this.state = state;
    this.statusEl = statusEl;
    this.overlayEl = overlayEl;
    this.court = readCourt(state?.rules);
    this.halfLength = this.court.length / 2;
    this.halfWidth = this.court.width / 2;
    this.players = new Map();
    this.frames = 0;
    this.visible = false;
    this.lastIndex = -1;
    this.lastTicks = null;
    this.lastTime = performance.now();
    this.clipCache = new Map();
    this.pixelSignature = 0;
    this.playerPosition = new THREE.Vector3();
    this.hoops = [];

    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      powerPreference: "high-performance",
    });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.75));
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 1.02;
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;

    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color(0x0a1018);

    this.camera = new THREE.OrthographicCamera(-52, 52, 34, -34, 0.1, 500);
    this.controls = new OrbitControls(this.camera, canvas);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.08;
    this.controls.enableRotate = false;
    this.controls.enablePan = true;
    this.controls.screenSpacePanning = true;
    this.controls.panSpeed = 0.9;
    this.controls.zoomSpeed = 0.9;
    this.controls.minZoom = 0.65;
    this.controls.maxZoom = 3.4;

    this.buildLights();
    this.buildCourt();
    this.buildHoops();
    this.buildBall();
    this.preset = "broadcast";
    this.applyPreset("broadcast");

    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(canvas.parentElement ?? canvas);
    this.resize();
  }

  buildLights() {
    const hemisphere = new THREE.HemisphereLight(0xe8f0fb, 0x241d16, 0.85);
    this.scene.add(hemisphere);

    const key = new THREE.DirectionalLight(0xfff4e2, 2.1);
    key.position.set(28, 74, 38);
    key.castShadow = true;
    key.shadow.mapSize.set(1024, 1024);
    key.shadow.camera.left = -78;
    key.shadow.camera.right = 78;
    key.shadow.camera.top = 56;
    key.shadow.camera.bottom = -56;
    key.shadow.camera.near = 20;
    key.shadow.camera.far = 190;
    key.shadow.bias = -0.0006;
    key.shadow.normalBias = 0.02;
    this.shadowCamera = key.shadow.camera;
    this.scene.add(key);

    const fill = new THREE.DirectionalLight(0xbfd4f2, 0.55);
    fill.position.set(-46, 36, -42);
    this.scene.add(fill);
  }

  buildCourt() {
    const apron = new THREE.Mesh(
      new THREE.PlaneGeometry(this.court.length + 46, this.court.width + 40),
      new THREE.MeshStandardMaterial({ color: 0x11161d, roughness: 0.55, metalness: 0.15 }),
    );
    apron.rotation.x = -Math.PI / 2;
    apron.position.y = -0.02;
    apron.receiveShadow = true;
    this.scene.add(apron);

    const floor = new THREE.Mesh(
      new THREE.PlaneGeometry(this.court.length, this.court.width),
      new THREE.MeshStandardMaterial({
        map: makeWoodTexture(),
        roughness: 0.78,
        metalness: 0.02,
      }),
    );
    floor.rotation.x = -Math.PI / 2;
    floor.receiveShadow = true;
    this.scene.add(floor);

    const logo = new THREE.Mesh(
      new THREE.CircleGeometry(5.6, 48),
      new THREE.MeshBasicMaterial({
        map: makeLogoTexture(),
        transparent: true,
        opacity: 0.55,
        depthWrite: false,
      }),
    );
    logo.rotation.x = -Math.PI / 2;
    logo.position.set(0, 0.012, 0);
    this.scene.add(logo);

    const lines = new LineBuilder();
    const halfLength = this.halfLength;
    const halfWidth = this.halfWidth;
    lines.segment(-halfLength, -halfWidth, halfLength, -halfWidth);
    lines.segment(-halfLength, halfWidth, halfLength, halfWidth);
    lines.segment(-halfLength, -halfWidth, -halfLength, halfWidth);
    lines.segment(halfLength, -halfWidth, halfLength, halfWidth);
    lines.segment(0, -halfWidth, 0, halfWidth);
    lines.circle(0, 0, CENTER_CIRCLE_FT);

    const laneHalf = LANE_WIDTH_FT / 2;
    for (const side of [-1, 1]) {
      const baseline = side * halfLength;
      const laneEnd = baseline - side * LANE_LENGTH_FT;
      lines.segment(baseline, -laneHalf, laneEnd, -laneHalf);
      lines.segment(baseline, laneHalf, laneEnd, laneHalf);
      lines.segment(laneEnd, -laneHalf, laneEnd, laneHalf);
      lines.circle(laneEnd, 0, FREE_THROW_CIRCLE_FT);

      const rimWorldX =
        side < 0
          ? -halfLength + this.court.leftHoop
          : -halfLength + this.court.rightHoop;
      const cornerZ = halfWidth - CORNER_THREE_FROM_SIDELINE_FT;
      const cornerEndX =
        side < 0
          ? rimWorldX + Math.sqrt(this.court.threePoint ** 2 - cornerZ ** 2)
          : rimWorldX - Math.sqrt(this.court.threePoint ** 2 - cornerZ ** 2);
      lines.segment(baseline, -cornerZ, cornerEndX, -cornerZ);
      lines.segment(baseline, cornerZ, cornerEndX, cornerZ);
      const angle = Math.acos(
        Math.min(1, Math.abs(cornerEndX - rimWorldX) / this.court.threePoint),
      );
      lines.arc(rimWorldX, 0, this.court.threePoint, side < 0 ? -angle : Math.PI - angle, side < 0 ? angle : Math.PI + angle);
      lines.arc(rimWorldX, 0, RESTRICTED_ARC_FT, side < 0 ? -Math.PI / 2 : Math.PI / 2, side < 0 ? Math.PI / 2 : (3 * Math.PI) / 2);
    }

    const lineMesh = new THREE.LineSegments(
      lines.toGeometry(),
      new THREE.LineBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.92 }),
    );
    lineMesh.position.y = 0.03;
    this.scene.add(lineMesh);

    this.buildPaint(-1, TEAM_COLORS.home);
    this.buildPaint(1, TEAM_COLORS.away);
    this.buildSidelineBoards();
  }

  buildSidelineBoards() {
    const body = new THREE.MeshStandardMaterial({ color: 0x161f2b, roughness: 0.55, metalness: 0.18 });
    const accent = new THREE.MeshStandardMaterial({
      color: 0x24384f,
      roughness: 0.32,
      metalness: 0.3,
      emissive: 0x0b1420,
      emissiveIntensity: 0.6,
    });
    const boardLength = this.court.length + 10;
    const boardHeight = 1.15;
    const offset = this.halfWidth + 4.5;
    for (const side of [-1, 1]) {
      const bodyMesh = new THREE.Mesh(
        new THREE.BoxGeometry(boardLength, boardHeight, 0.5),
        body,
      );
      bodyMesh.position.set(0, boardHeight / 2, side * offset);
      bodyMesh.castShadow = true;
      bodyMesh.receiveShadow = true;
      this.scene.add(bodyMesh);
      const accentMesh = new THREE.Mesh(
        new THREE.BoxGeometry(boardLength, 0.14, 0.56),
        accent,
      );
      accentMesh.position.set(0, boardHeight + 0.06, side * offset);
      this.scene.add(accentMesh);
    }
    const endLength = this.court.width + 8;
    for (const side of [-1, 1]) {
      const endX = side * (this.halfLength + 5.5);
      const bodyMesh = new THREE.Mesh(new THREE.BoxGeometry(0.5, boardHeight, endLength), body);
      bodyMesh.position.set(endX, boardHeight / 2, 0);
      bodyMesh.castShadow = true;
      this.scene.add(bodyMesh);
      const accentMesh = new THREE.Mesh(new THREE.BoxGeometry(0.56, 0.14, endLength), accent);
      accentMesh.position.set(endX, boardHeight + 0.06, 0);
      this.scene.add(accentMesh);
    }
  }

  buildPaint(side, color) {
    const laneMesh = new THREE.Mesh(
      new THREE.PlaneGeometry(LANE_LENGTH_FT, LANE_WIDTH_FT),
      new THREE.MeshBasicMaterial({
        color,
        transparent: true,
        opacity: 0.24,
        depthWrite: false,
      }),
    );
    laneMesh.rotation.x = -Math.PI / 2;
    laneMesh.position.set(side * (this.halfLength - LANE_LENGTH_FT / 2), 0.008, 0);
    this.scene.add(laneMesh);
  }

  buildHoops() {
    const poleMaterial = new THREE.MeshStandardMaterial({ color: 0x6d7889, roughness: 0.5, metalness: 0.6 });
    const boardMaterial = new THREE.MeshStandardMaterial({
      color: 0xd8e2ef,
      roughness: 0.25,
      metalness: 0.05,
      transparent: true,
      opacity: 0.86,
    });
    const rimMaterial = new THREE.MeshStandardMaterial({ color: 0xe2622a, roughness: 0.35, metalness: 0.55 });

    for (const side of [-1, 1]) {
      const baseline = side * this.halfLength;
      const rimX = side < 0 ? -this.halfLength + this.court.leftHoop : -this.halfLength + this.court.rightHoop;
      const backboardX = baseline - side * BACKBOARD_FROM_BASELINE_FT;
      const poleX = baseline + side * 2.4;
      this.hoops.push({ side, baseline, rimX, backboardX, poleX, rimY: RIM_HEIGHT_FT });

      const pole = new THREE.Mesh(new THREE.CylinderGeometry(0.32, 0.36, 13, 16), poleMaterial);
      pole.position.set(poleX, 6.5, 0);
      pole.castShadow = true;
      this.scene.add(pole);

      const arm = new THREE.Mesh(
        new THREE.BoxGeometry(Math.abs(poleX - backboardX) + 0.4, 0.32, 0.32),
        poleMaterial,
      );
      arm.position.set((poleX + backboardX) / 2, 13, 0);
      arm.castShadow = true;
      this.scene.add(arm);

      const board = new THREE.Mesh(
        new THREE.BoxGeometry(0.12, BACKBOARD_HEIGHT_FT, BACKBOARD_WIDTH_FT),
        boardMaterial,
      );
      board.position.set(backboardX, BACKBOARD_BOTTOM_FT + BACKBOARD_HEIGHT_FT / 2, 0);
      board.castShadow = true;
      this.scene.add(board);

      const inner = new THREE.Mesh(
        new THREE.PlaneGeometry(1.9, 1.4),
        new THREE.MeshBasicMaterial({ color: 0x2a3542, transparent: true, opacity: 0.55 }),
      );
      inner.rotation.y = side < 0 ? Math.PI / 2 : -Math.PI / 2;
      inner.position.set(backboardX - side * 0.08, RIM_HEIGHT_FT + 0.55, 0);
      this.scene.add(inner);

      const rim = new THREE.Mesh(new THREE.TorusGeometry(RIM_RADIUS_FT, 0.055, 10, 36), rimMaterial);
      rim.rotation.x = Math.PI / 2;
      rim.position.set(rimX, RIM_HEIGHT_FT, 0);
      rim.castShadow = true;
      this.scene.add(rim);

      const netLines = new LineBuilder();
      const spokes = 12;
      const netBottom = RIM_HEIGHT_FT - 1.4;
      const netRadius = 0.44;
      for (let spoke = 0; spoke < spokes; spoke += 1) {
        const angle = (spoke / spokes) * Math.PI * 2;
        const nextAngle = ((spoke + 1) / spokes) * Math.PI * 2;
        netLines.segment3(
          rimX + Math.cos(angle) * RIM_RADIUS_FT,
          RIM_HEIGHT_FT,
          Math.sin(angle) * RIM_RADIUS_FT,
          rimX + Math.cos(angle) * netRadius,
          netBottom,
          Math.sin(angle) * netRadius,
        );
        netLines.segment3(
          rimX + Math.cos(angle) * netRadius,
          netBottom,
          Math.sin(angle) * netRadius,
          rimX + Math.cos(nextAngle) * netRadius,
          netBottom,
          Math.sin(nextAngle) * netRadius,
        );
      }
      const net = new THREE.LineSegments(
        netLines.toGeometry(),
        new THREE.LineBasicMaterial({ color: 0xf4f7ff, transparent: true, opacity: 0.55 }),
      );
      this.scene.add(net);
    }
  }

  buildBall() {
    const group = new THREE.Group();
    const ball = new THREE.Mesh(
      new THREE.SphereGeometry(BALL_RADIUS_FT, 24, 18),
      new THREE.MeshStandardMaterial({ color: 0xd4691f, roughness: 0.65, metalness: 0.02 }),
    );
    ball.castShadow = true;
    group.add(ball);
    const seamMaterial = new THREE.MeshBasicMaterial({ color: 0x2b1508 });
    for (const rotation of [
      [0, 0, 0],
      [Math.PI / 2, 0, 0],
      [0, Math.PI / 2, 0],
    ]) {
      const seam = new THREE.Mesh(new THREE.TorusGeometry(BALL_RADIUS_FT * 0.995, 0.018, 6, 32), seamMaterial);
      seam.rotation.set(rotation[0], rotation[1], rotation[2]);
      group.add(seam);
    }
    this.ballGroup = group;
    this.ballMesh = ball;
    this.scene.add(group);

    this.ballTrail = [];
    this.trailGeometry = new THREE.BufferGeometry();
    this.trailGeometry.setAttribute(
      "position",
      new THREE.BufferAttribute(new Float32Array(BALL_TRAIL_POINTS * 3), 3),
    );
    this.trailGeometry.setDrawRange(0, 0);
    this.trailLine = new THREE.Line(
      this.trailGeometry,
      new THREE.LineBasicMaterial({
        color: 0xffb066,
        transparent: true,
        opacity: 0.42,
        depthWrite: false,
      }),
    );
    this.trailLine.frustumCulled = false;
    this.trailLine.visible = false;
    this.scene.add(this.trailLine);
  }

  async loadPlayers(url) {
    const loader = new GLTFLoader();
    const gltf = await loader.loadAsync(url);
    const template = gltf.scene;
    template.updateMatrixWorld(true);
    this.modelMetrics = measureModel(template);
    const rest = collectRestPose(template);
    this.clips = gltf.animations.map((clip) => completeClip(clip, rest, this.clipCache));
    this.template = template;
    if (this.overlayEl) this.overlayEl.hidden = true;
    this.publishStatus("ready", "true");
  }

  applyPreset(name) {
    const key = PRESETS[name] ? name : "broadcast";
    const preset = PRESETS[key];
    this.preset = key;
    this.publishStatus("preset", key);
    this.camera.zoom = 1;
    if (key === "follow") {
      this.controls.enabled = false;
      this.followTarget = this.followTarget ?? new THREE.Vector3(0, 2.5, 0);
      this.placeCamera(preset, this.followTarget);
    } else {
      this.controls.enabled = true;
      this.controls.target.set(preset.target[0], preset.target[1], preset.target[2]);
      this.placeCamera(preset, this.controls.target);
      this.controls.update();
    }
    this.fitCamera(preset);
  }

  // 正交平行投影的固定俯角：同一高度的球员在画面里尺寸一致，
  // 球场不产生透视收缩，整体保持 2.5D 的斜视效果。
  placeCamera(preset, target) {
    const elevation = THREE.MathUtils.degToRad(preset.elevation);
    const azimuth = THREE.MathUtils.degToRad(preset.azimuth);
    const distance = 160;
    const direction = new THREE.Vector3(
      Math.sin(azimuth) * Math.cos(elevation),
      Math.sin(elevation),
      Math.cos(azimuth) * Math.cos(elevation),
    );
    this.camera.position.copy(target).addScaledVector(direction, distance);
    this.camera.up.set(0, 1, 0);
    this.camera.lookAt(target);
    this.camera.updateMatrixWorld(true);
  }

  updateFollowCamera(delta) {
    const preset = PRESETS.follow;
    if (!this.followTarget) this.followTarget = new THREE.Vector3(0, 2.5, 0);
    const focus = this.ballGroup.visible ? this.ballGroup.position : new THREE.Vector3(0, 0, 0);
    const smoothing = Math.min(1, delta * 4);
    this.followTarget.x += (focus.x - this.followTarget.x) * smoothing;
    this.followTarget.z += (focus.z - this.followTarget.z) * smoothing;
    this.followTarget.y = 2.5;
    this.placeCamera(preset, this.followTarget);
  }

  fitCamera(preset) {
    const aspect = Math.max(0.4, this.canvas.clientWidth / Math.max(1, this.canvas.clientHeight));
    const halfWidth = preset.coverage / 2;
    const halfHeight = halfWidth / aspect;
    this.camera.left = -halfWidth;
    this.camera.right = halfWidth;
    this.camera.top = halfHeight;
    this.camera.bottom = -halfHeight;
    this.camera.updateProjectionMatrix();
  }

  clampPan() {
    if (!this.controls.enabled) return;
    const limitX = this.halfLength - 12;
    const limitZ = this.halfWidth - 6;
    const target = this.controls.target;
    const clampedX = THREE.MathUtils.clamp(target.x, -limitX, limitX);
    const clampedZ = THREE.MathUtils.clamp(target.z, -limitZ, limitZ);
    if (clampedX === target.x && clampedZ === target.z) return;
    const offset = new THREE.Vector3(clampedX - target.x, 0, clampedZ - target.z);
    target.add(offset);
    this.camera.position.add(offset);
  }

  cyclePreset() {
    const order = ["broadcast", "follow", "top"];
    const next = order[(order.indexOf(this.preset) + 1) % order.length];
    this.applyPreset(next);
    return next;
  }

  resize() {
    const width = Math.max(1, this.canvas.clientWidth);
    const height = Math.max(1, this.canvas.clientHeight);
    this.renderer.setSize(width, height, false);
    this.fitCamera(PRESETS[this.preset] ?? PRESETS.broadcast);
  }

  setVisible(visible) {
    this.visible = visible;
    this.canvas.style.display = visible ? "block" : "none";
    if (visible) {
      this.lastTime = performance.now();
      this.resize();
    }
  }

  playerView(player) {
    let view = this.players.get(player.id);
    if (view) return view;
    if (!this.template) return null;
    view = new PlayerFigure({
      template: this.template,
      clips: this.clips,
      team: player.team,
      numberTexture: makeNumberTexture(player.jersey, TEAM_COLORS[player.team] ?? TEAM_COLORS.home),
      metrics: this.modelMetrics,
    });
    this.players.set(player.id, view);
    this.scene.add(view.root);
    return view;
  }

  frameAt(index) {
    const ticks = this.state?.ticks;
    if (!ticks || !ticks.length) return null;
    const clamped = Math.min(ticks.length - 1, Math.max(0, index));
    return ticks[clamped];
  }

  syncPlayers(frame, nextFrame, alpha, delta) {
    const courtLength = this.court.length;
    const courtWidth = this.court.width;
    const tickSeconds = finite(this.state?.rules?.tick_seconds, 0.05);
    const seen = new Set();
    for (const player of frame.players ?? []) {
      if (player.onCourt === false) continue;
      seen.add(player.id);
      const view = this.playerView(player);
      if (!view) continue;
      const nextPlayer = (nextFrame.players ?? []).find((candidate) => candidate.id === player.id) ?? player;
      const x = THREE.MathUtils.lerp(finite(player.x, 0.5), finite(nextPlayer.x, player.x ?? 0.5), alpha);
      const y = THREE.MathUtils.lerp(finite(player.y, 0.5), finite(nextPlayer.y, player.y ?? 0.5), alpha);
      const worldX = (x - 0.5) * courtLength;
      const worldZ = (y - 0.5) * courtWidth;
      const facingX = THREE.MathUtils.lerp(finite(player.facing_x, 0), finite(nextPlayer.facing_x, player.facing_x ?? 0), alpha);
      const facingY = THREE.MathUtils.lerp(finite(player.facing_y, 0), finite(nextPlayer.facing_y, player.facing_y ?? 0), alpha);
      const dx = (finite(nextPlayer.x, player.x ?? 0.5) - finite(player.x, 0.5)) * courtLength;
      const dy = (finite(nextPlayer.y, player.y ?? 0.5) - finite(player.y, 0.5)) * courtWidth;
      view.speed = this.state?.playing ? Math.hypot(dx, dy) / tickSeconds : 0;
      const hasFacing = Math.hypot(facingX, facingY) > 0.05;
      const directionX = hasFacing ? facingX : dx;
      const directionY = hasFacing ? facingY : dy;
      let yaw = view.yaw;
      if (Math.hypot(directionX, directionY) > 0.05) {
        // 固定正交镜头下保留真实朝向：背对镜头就展示背面。
        yaw = Math.atan2(directionX, directionY);
      }
      view.place(worldX, worldZ, yaw, delta);
      view.root.visible = true;
    }
    for (const [id, view] of this.players) {
      if (!seen.has(id)) view.root.visible = false;
    }
  }

  syncBall(frame, nextFrame, alpha, delta) {
    const ball = frame.ball;
    if (!ball) {
      this.ballGroup.visible = false;
      return;
    }
    this.ballGroup.visible = true;
    const courtLength = this.court.length;
    const courtWidth = this.court.width;
    const nextBall = nextFrame.ball ?? ball;
    const x = THREE.MathUtils.lerp(finite(ball.x, 0.5), finite(nextBall.x, ball.x ?? 0.5), alpha);
    const y = THREE.MathUtils.lerp(finite(ball.y, 0.5), finite(nextBall.y, ball.y ?? 0.5), alpha);
    const z = THREE.MathUtils.lerp(finite(ball.z, 4), finite(nextBall.z, ball.z ?? 4), alpha);
    const worldX = (x - 0.5) * courtLength;
    const worldZ = (y - 0.5) * courtWidth;
    const holderId = ball.holderId ?? null;
    const holder = holderId ? this.players.get(holderId) : null;
    let targetX = worldX;
    let targetZ = worldZ;
    let targetY = Math.max(BALL_RADIUS_FT, z);
    if (ball.status === "held" && holder?.fist) {
      holder.fist.getWorldPosition(this.playerPosition);
      targetX = this.playerPosition.x;
      targetY = this.playerPosition.y + 0.32;
      targetZ = this.playerPosition.z;
    }
    // 协议的高度是离散档位（持球 4 英尺、传球 6 英尺、投篮 12 英尺），
    // 直接跳变会让球在一帧内瞬移；按真实帧间隔平滑逼近目标高度。
    if (this.state?.playing && this.ballHeight !== undefined) {
      this.ballHeight = THREE.MathUtils.damp(this.ballHeight, targetY, 9, Math.min(0.1, delta));
    } else {
      this.ballHeight = targetY;
    }
    this.ballGroup.position.set(targetX, this.ballHeight, targetZ);
    const travel = Math.hypot(
      this.ballGroup.position.x - this.lastBallX,
      this.ballGroup.position.y - this.lastBallY,
      this.ballGroup.position.z - this.lastBallZ,
    );
    this.lastBallX = this.ballGroup.position.x;
    this.lastBallY = this.ballGroup.position.y;
    this.lastBallZ = this.ballGroup.position.z;
    this.ballMesh.rotation.x += travel * 1.6;
    this.ballMesh.rotation.z += travel * 0.9;
    this.updateBallTrail(ball.status === "held");
  }

  updateBallTrail(held) {
    if (held) {
      this.ballTrail.length = 0;
    } else {
      const last = this.ballTrail[this.ballTrail.length - 1];
      if (!last || last.distanceToSquared(this.ballGroup.position) > 0.09) {
        this.ballTrail.push(this.ballGroup.position.clone());
        if (this.ballTrail.length > BALL_TRAIL_POINTS) this.ballTrail.shift();
      }
    }
    const attribute = this.trailGeometry.getAttribute("position");
    for (let index = 0; index < BALL_TRAIL_POINTS; index += 1) {
      const point = this.ballTrail[index] ?? this.ballGroup.position;
      attribute.setXYZ(index, point.x, point.y, point.z);
    }
    attribute.needsUpdate = true;
    this.trailGeometry.setDrawRange(0, this.ballTrail.length);
    this.trailLine.visible = this.ballTrail.length > 1;
  }

  triggerEvents(frame) {
    if (!this.state?.playing) return;
    if (!frame) return;
    const events = new Set();
    if (frame.eventType) events.add(String(frame.eventType));
    for (const event of frame.events ?? []) events.add(String(event));
    if (!events.size) return;

    const holderId = frame.ball?.holderId ?? null;
    const nearest = this.nearestPlayerToBall(frame);

    if (events.has("SHOT") || events.has("FREE_THROW")) {
      const shooter = holderId ?? nearest;
      if (shooter) this.players.get(shooter)?.playGesture("shot");
    } else if (events.has("PASS")) {
      if (holderId) this.players.get(holderId)?.playGesture("pass");
    } else if (events.has("REBOUND")) {
      if (nearest) this.players.get(nearest)?.playGesture("rebound");
    }
  }

  detectScore(frame) {
    const score = frame.score ?? {};
    const home = finite(score.home, 0);
    const away = finite(score.away, 0);
    const previous = this.scoreSeen;
    if (previous && this.state?.playing) {
      const team = home > previous.home ? "home" : away > previous.away ? "away" : null;
      if (team) {
        for (const [id, view] of this.players) {
          const player = (frame.players ?? []).find((candidate) => candidate.id === id);
          if (player?.team === team) view.playGesture("celebrate");
        }
      }
    }
    this.scoreSeen = { home, away };
  }

  nearestPlayerToBall(frame) {
    const ball = frame.ball;
    if (!ball) return null;
    let best = null;
    let bestDistance = Infinity;
    for (const player of frame.players ?? []) {
      if (player.onCourt === false) continue;
      const distance = Math.hypot(finite(player.x, 0.5) - finite(ball.x, 0.5), finite(player.y, 0.5) - finite(ball.y, 0.5));
      if (distance < bestDistance) {
        bestDistance = distance;
        best = player.id;
      }
    }
    return best;
  }

  sync(delta, now) {
    const state = this.state;
    if (!state || !state.ticks || !state.ticks.length) {
      this.ballGroup.visible = false;
      for (const view of this.players.values()) view.root.visible = false;
      return;
    }
    if (this.lastTicks !== state.ticks) {
      this.lastTicks = state.ticks;
      this.lastIndex = -1;
      this.scoreSeen = null;
      this.ballHeight = undefined;
      this.ballTrail.length = 0;
      for (const view of this.players.values()) {
        view.speed = 0;
        view.gesture = null;
      }
    }
    const index = Math.min(state.ticks.length - 1, Math.max(0, state.idx | 0));
    const frame = this.frameAt(index);
    const nextIndex = Math.min(state.ticks.length - 1, index + 1);
    const nextFrame = this.frameAt(nextIndex);
    const alpha = state.playing ? Math.min(1, Math.max(0, finite(state.accumulator, 0))) : 0;
    this.syncPlayers(frame, nextFrame, alpha, delta);
    if (state.playing && index === this.lastIndex + 1) this.triggerEvents(frame);
    this.detectScore(frame);
    for (const view of this.players.values()) {
      if (view.root.visible) view.update(delta, now);
    }
    this.syncBall(frame, nextFrame, alpha, delta);
    this.lastIndex = index;
    this.publishClipState(frame);
  }

  publishClipState(frame) {
    if (!this.statusEl || !frame) return;
    const holderId = frame.ball?.holderId ?? null;
    const view = holderId ? this.players.get(holderId) : null;
    const clip = view?.current?.getClip().name ?? "";
    if (this.statusEl.dataset.clip !== clip) this.statusEl.dataset.clip = clip;
    this.statusEl.dataset.index = String(this.state?.idx ?? 0);
  }

  publishStatus(key, value) {
    if (!this.statusEl) return;
    this.statusEl.dataset[key] = value;
  }

  publishMetrics() {
    if (!this.statusEl) return;
    this.statusEl.dataset.renders = String(this.frames);
    this.statusEl.dataset.players = String(this.players.size);
    this.statusEl.dataset.visiblePlayers = String(
      [...this.players.values()].filter((view) => view.root.visible).length,
    );
    this.statusEl.dataset.preset = this.preset;
    const gl = this.renderer.getContext();
    const width = 96;
    const height = 48;
    const buffer = new Uint8Array(width * height * 4);
    const x = Math.max(0, Math.floor(gl.drawingBufferWidth / 2 - width / 2));
    const y = Math.max(0, Math.floor(gl.drawingBufferHeight / 2 - height / 2));
    gl.readPixels(x, y, width, height, gl.RGBA, gl.UNSIGNED_BYTE, buffer);
    const colors = new Set();
    let total = 0;
    for (let i = 0; i < buffer.length; i += 4) {
      colors.add((buffer[i] >> 3) << 10 | (buffer[i + 1] >> 3) << 5 | buffer[i + 2] >> 3);
      total += buffer[i] + buffer[i + 1] + buffer[i + 2];
    }
    this.pixelSignature = colors.size;
    this.statusEl.dataset.pixelColors = String(colors.size);
    this.statusEl.dataset.pixelMean = String(Math.round(total / (buffer.length / 4) / 3));
  }

  start() {
    const loop = (now) => {
      this.rafId = requestAnimationFrame(loop);
      const delta = Math.min(0.1, (now - this.lastTime) / 1000);
      this.lastTime = now;
      if (!this.visible) return;
      this.sync(delta, now);
      if (this.preset === "follow") this.updateFollowCamera(delta);
      else {
        this.controls.update();
        this.clampPan();
      }
      this.renderer.render(this.scene, this.camera);
      this.frames += 1;
      if (this.frames === 1 || this.frames === 6 || this.frames % 90 === 0) this.publishMetrics();
    };
    this.rafId = requestAnimationFrame(loop);
  }

  dispose() {
    cancelAnimationFrame(this.rafId);
    this.resizeObserver.disconnect();
    this.controls.dispose();
    this.renderer.dispose();
  }
}

export async function mountCourt3d({ canvas, state, statusEl, overlayEl, assetUrl }) {
  const court = new Court3D({ canvas, state, statusEl, overlayEl });
  await court.loadPlayers(assetUrl);
  court.setVisible(true);
  court.start();
  return court;
}
