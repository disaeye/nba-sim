// 2.5D 球场视图的无头端到端验证。
//
// 通过 Chrome DevTools Protocol 启动真实浏览器，加载页面，等待球场场景就绪，
// 读取渲染统计、场景图与像素采样，并检查控制台异常。
// 依赖：本地调试服务器（默认 http://127.0.0.1:31100/）。

import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";

const URL = process.env.COURT3D_URL ?? "http://127.0.0.1:31100/";
const PORT = Number(process.env.COURT3D_CDP_PORT ?? 9333);
const PROFILE_DIR = ".work/court3d/chrome-profile";
const failures = [];

function chromeBinary() {
  const candidates = [process.env.CHROME_BIN, "google-chrome", "chromium", "chromium-browser"].filter(Boolean);
  for (const candidate of candidates) {
    try {
      execFileSync("which", [candidate], { stdio: "ignore" });
      return candidate;
    } catch {
      continue;
    }
  }
  throw new Error("未找到 Chrome 或 Chromium 可执行文件");
}

function check(label, condition, detail) {
  if (condition) {
    console.log(`✅ ${label}${detail ? ` · ${detail}` : ""}`);
  } else {
    console.log(`❌ ${label}${detail ? ` · ${detail}` : ""}`);
    failures.push(label);
  }
}

async function waitFor(probe, timeoutMs, intervalMs = 250) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await probe();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
  return null;
}

class Devtools {
  constructor(webSocketUrl) {
    this.nextId = 1;
    this.pending = new Map();
    this.consoleErrors = [];
    this.exceptions = [];
    this.socket = new WebSocket(webSocketUrl);
    this.ready = new Promise((resolve, reject) => {
      this.socket.addEventListener("open", () => resolve());
      this.socket.addEventListener("error", (event) => reject(event.error ?? new Error("WS 失败")));
    });
    this.socket.addEventListener("message", (event) => this.dispatch(JSON.parse(event.data)));
  }

  dispatch(message) {
    if (message.id && this.pending.has(message.id)) {
      const { resolve, reject } = this.pending.get(message.id);
      this.pending.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
      return;
    }
    if (message.method === "Runtime.exceptionThrown") {
      this.exceptions.push(message.params.exceptionDetails.text);
    }
    if (message.method === "Runtime.consoleAPICalled" && message.params.type === "error") {
      this.consoleErrors.push(
        message.params.args.map((arg) => arg.value ?? arg.description ?? "").join(" "),
      );
    }
    if (message.method === "Log.entryAdded" && message.params.entry.level === "error") {
      this.consoleErrors.push(message.params.entry.text);
    }
  }

  async send(method, params = {}) {
    await this.ready;
    const id = this.nextId;
    this.nextId += 1;
    const payload = JSON.stringify({ id, method, params });
    const promise = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.socket.send(payload);
    return promise;
  }

  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.text);
    }
    return result.result.value;
  }

  close() {
    this.socket.close();
  }
}

async function main() {
  mkdirSync(PROFILE_DIR, { recursive: true });
  const chrome = spawn(
    chromeBinary(),
    [
      "--headless=new",
      "--no-sandbox",
      "--disable-dev-shm-usage",
      "--enable-unsafe-swiftshader",
      "--use-gl=angle",
      "--use-angle=swiftshader",
      "--window-size=1440,900",
      `--remote-debugging-port=${PORT}`,
      `--user-data-dir=${PROFILE_DIR}`,
      "about:blank",
    ],
    { stdio: "ignore" },
  );

  let devtools = null;
  try {
    const version = await waitFor(async () => {
      const response = await fetch(`http://127.0.0.1:${PORT}/json/version`).catch(() => null);
      return response ? response.json() : null;
    }, 20000);
    check("DevTools 协议可用", Boolean(version), version?.Browser?.split("/")[0]);

    const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
    const page = targets.find((target) => target.type === "page");
    if (!page) throw new Error("没有可用的页面目标");

    devtools = new Devtools(page.webSocketDebuggerUrl);
    await devtools.send("Runtime.enable");
    await devtools.send("Log.enable");
    await devtools.send("Page.enable");
    await devtools.send("Page.navigate", { url: URL });

    const probeSamples = [];
    const ready = await waitFor(async () => {
      const state = await devtools
        .evaluate(
          `(() => { const el = document.getElementById("court3dStatus"); return el ? el.getAttribute("data-ready") : "no-element"; })()`,
        )
        .catch((error) => `probe-error:${error.message}`);
      probeSamples.push(state);
      return state === "true";
    }, 120000, 500);
    check("球场场景就绪", Boolean(ready));
    if (!ready) {
      console.log("探测样本：", JSON.stringify(probeSamples.slice(0, 5)));
    }

    if (!ready) {
      const diagnostic = await devtools
        .evaluate(`({
          readyState: document.readyState,
          href: location.href,
          runStatus: document.getElementById("runStatus")?.textContent ?? null,
          ticks: window.__nbaDebug?.ticks?.length ?? null,
          viewMode: window.__nbaDebug?.viewMode ?? null,
          status: { ...(document.getElementById("court3dStatus")?.dataset ?? {}) },
          overlayHidden: document.getElementById("court3dLoading")?.hidden ?? null,
          hasMount: typeof window.__nbaDebug?.court3d,
        })`)
        .catch((error) => ({ error: String(error) }));
      console.log("诊断：", JSON.stringify(diagnostic));
    }

    if (ready) {
      await waitFor(async () => {
        const value = await devtools
          .evaluate(`document.getElementById("court3dStatus").getAttribute("data-pixel-colors")`)
          .catch(() => null);
        return value;
      }, 60000, 400);
      const stats = await devtools.evaluate(`(() => {
        const status = { ...document.getElementById("court3dStatus").dataset };
        const court = window.__nbaDebug?.court3d;
        const scene = court?.scene;
        let skinned = 0;
        let lineSegments = 0;
        let torus = 0;
        scene?.traverse((object) => {
          if (object.isSkinnedMesh) skinned += 1;
          if (object.isLineSegments) lineSegments += 1;
          if (object.geometry?.type === "TorusGeometry") torus += 1;
        });
        return {
          status,
          players: court?.players.size ?? 0,
          visible: [...(court?.players.values() ?? [])].filter((view) => view.root.visible).length,
          skinned,
          lineSegments,
          torus,
          ballVisible: court?.ballGroup.visible ?? false,
          frames: court?.frames ?? 0,
          viewMode: window.__nbaDebug?.viewMode,
          ticks: window.__nbaDebug?.ticks.length ?? 0,
          clips: [...(court?.players.values() ?? [])]
            .map((view) => view.current?.getClip().name ?? null)
            .filter(Boolean),
        };
      })()`);

      check("十名球员模型已加载", stats.players === 10, `players=${stats.players}`);
      check("在场球员全部可见", stats.visible === 10, `visible=${stats.visible}`);
      check("蒙皮网格已实例化", stats.skinned >= 40, `skinned=${stats.skinned}`);
      check("球场地线与篮圈已构建", stats.lineSegments >= 2 && stats.torus >= 4,
        `lines=${stats.lineSegments} torus=${stats.torus}`);
      check("篮球可见", stats.ballVisible === true);
      check("渲染循环已启动", stats.frames >= 1, `frames=${stats.frames}`);
      check("场景像素非单色", Number(stats.status.pixelColors ?? 0) >= 12,
        `colors=${stats.status.pixelColors} mean=${stats.status.pixelMean}`);
      check("至少一种动画在播放", stats.clips.length > 0, `clips=${[...new Set(stats.clips)].join(",")}`);
      check("默认使用 2.5D 球场", stats.viewMode === "3d", `mode=${stats.viewMode}`);

      const geometry = await devtools.evaluate(`(() => {
        const court = window.__nbaDebug.court3d;
        const state = window.__nbaDebug;
        const frame = state.ticks[state.idx];
        const onCourt = (frame.players || []).filter((item) => item.onCourt !== false);
        const nearest = onCourt
          .map((item) => ({ item, view: court.players.get(item.id) }))
          .filter((entry) => entry.view?.root.visible)
          .sort(
            (a, b) =>
              a.view.root.position.distanceTo(court.ballGroup.position) -
              b.view.root.position.distanceTo(court.ballGroup.position),
          )[0];
        const player = nearest?.item ?? null;
        const view = nearest?.view ?? null;
        const nextFrame = state.ticks[Math.min(state.ticks.length - 1, state.idx + 1)];
        const nextPlayer = player
          ? (nextFrame.players || []).find((item) => item.id === player.id) ?? player
          : null;
        const alpha = state.playing ? Math.min(1, Math.max(0, state.accumulator || 0)) : 0;
        const expectedX = player
          ? ((player.x + (nextPlayer.x - player.x) * alpha) - 0.5) * court.court.length
          : null;
        const expectedZ = player
          ? ((player.y + (nextPlayer.y - player.y) * alpha) - 0.5) * court.court.width
          : null;
        const positionError = view && player
          ? Math.hypot(view.root.position.x - expectedX, view.root.position.z - expectedZ)
          : null;
        const halfLength = court.court.length / 2;
        const halfWidth = court.court.width / 2;
        const corners = [
          [-halfLength, 0, -halfWidth],
          [halfLength, 0, -halfWidth],
          [-halfLength, 0, halfWidth],
          [halfLength, 0, halfWidth],
        ].map(([x, y, z]) => {
          const projected = court.ballGroup.position.clone().set(x, y, z).project(court.camera);
          return [projected.x, projected.y, projected.z];
        });
        let handDistance = null;
        if (frame.ball?.status === "held" && frame.ball.holderId) {
          const holder = court.players.get(frame.ball.holderId);
          if (holder?.fist) {
            const hand = holder.fist.getWorldPosition(court.ballGroup.position.clone());
            handDistance = court.ballGroup.position.distanceTo(hand);
          }
        }
        const figureSizes = [...court.players.values()]
          .filter((item) => item.root.visible)
          .map((item) => {
            const head = item.model.getObjectByName("Ear3L") ?? item.model.getObjectByName("Head");
            const foot = item.model.getObjectByName("FootR");
            if (!head || !foot) return null;
            const headProjected = head
              .getWorldPosition(court.ballGroup.position.clone())
              .project(court.camera);
            const footProjected = foot
              .getWorldPosition(court.ballGroup.position.clone())
              .project(court.camera);
            return Math.abs(headProjected.y - footProjected.y) * court.canvas.clientHeight * 0.5;
          })
          .filter((size) => size !== null)
          .sort((a, b) => a - b);
        const columnSpans = [...court.players.values()]
          .filter((item) => item.root.visible)
          .map((item) => {
            const base = court.ballGroup.position
              .clone()
              .set(item.root.position.x, 0, item.root.position.z)
              .project(court.camera);
            const top = court.ballGroup.position
              .clone()
              .set(item.root.position.x, 7, item.root.position.z)
              .project(court.camera);
            return Math.abs(base.y - top.y);
          })
          .sort((a, b) => a - b);
        return {
          positionError,
          corners,
          handDistance,
          orthographic: court.camera.isOrthographicCamera === true,
          frustum: (court.camera.top - court.camera.bottom) / court.camera.zoom,
          projectionSpread: columnSpans.length
            ? columnSpans[columnSpans.length - 1] / columnSpans[0]
            : null,
          assetAlignment: (() => {
            const foot = court.template?.getObjectByName("FootR");
            const toe = court.template?.getObjectByName("FootR_end");
            if (!foot || !toe) return null;
            const from = foot.getWorldPosition(court.ballGroup.position.clone());
            const to = toe.getWorldPosition(court.ballGroup.position.clone());
            const angle =
              Math.atan2(to.x - from.x, to.z - from.z) +
              court.modelMetrics.forwardOffset;
            return Math.abs(Math.atan2(Math.sin(angle), Math.cos(angle)));
          })(),
          renderLowest: (() => {
            if (!view) return null;
            let lowest = Infinity;
            const vertex = court.ballGroup.position.clone();
            view.model.traverse((object) => {
              if (!object.isSkinnedMesh) return;
              const position = object.geometry?.attributes?.position;
              if (!position) return;
              const step = Math.max(1, Math.floor(position.count / 300));
              for (let index = 0; index < position.count; index += step) {
                object.getVertexPosition(index, vertex);
                object.localToWorld(vertex);
                lowest = Math.min(lowest, vertex.y);
              }
            });
            return Number.isFinite(lowest) ? Number(lowest.toFixed(3)) : null;
          })(),
          headHeight: (() => {
            if (!view) return null;
            const head = view.model.getObjectByName("Head");
            if (!head) return null;
            return Number(
              head.getWorldPosition(court.ballGroup.position.clone()).y.toFixed(3),
            );
          })(),
          facingError: (() => {
            if (!view || !player) return null;
            if (Math.hypot(player.facing_x || 0, player.facing_y || 0) < 0.2) return null;
            const target = Math.atan2(player.facing_x, player.facing_y);
            let delta = (view.targetYaw - target) % (Math.PI * 2);
            if (delta > Math.PI) delta -= Math.PI * 2;
            if (delta < -Math.PI) delta += Math.PI * 2;
            return Math.abs(delta);
          })(),
          facingSample: (() => {
            const players = (frame.players || []).filter((item) => item.onCourt !== false);
            const shortest = (angle) => {
              let value = angle % (Math.PI * 2);
              if (value > Math.PI) value -= Math.PI * 2;
              if (value < -Math.PI) value += Math.PI * 2;
              return value;
            };
            let away = 0;
            let backToCamera = 0;
            let lag = 0;
            for (const item of players) {
              const figure = court.players.get(item.id);
              if (!figure?.root.visible) continue;
              lag = Math.max(
                lag,
                Math.abs(
                  shortest(
                    figure.model.rotation.y - figure.forwardOffset - figure.targetYaw,
                  ),
                ),
              );
              if ((item.facing_y || 0) < -0.5) {
                away += 1;
                // 背向镜头的目标偏航应该落在远离正面（0）的半球。
                if (Math.abs(shortest(figure.targetYaw)) > Math.PI / 2 - 0.15) {
                  backToCamera += 1;
                }
              }
            }
            return { away, backToCamera, lag };
          })(),
          figurePixels: columnSpans.length
            ? columnSpans[Math.floor(columnSpans.length / 2)] * court.canvas.clientHeight * 0.5
            : null,
          posePixels: figureSizes.length ? figureSizes[Math.floor(figureSizes.length / 2)] : null,
          labelHeight: view?.label?.position.y ?? null,
          hoops: court.hoops,
          materials: (() => {
            let toon = 0;
            court.scene.traverse((object) => {
              const material = object.material;
              if (!material) return;
              for (const entry of Array.isArray(material) ? material : [material]) {
                if (entry.type === "MeshToonMaterial") toon += 1;
              }
            });
            return { toon };
          })(),
          shadowCoverage: (() => {
            const shadow = court.shadowCamera;
            if (!shadow) return null;
            shadow.updateMatrixWorld(true);
            const inverse = shadow.matrixWorld.clone().invert();
            let maxX = 0;
            let maxY = 0;
            let minDepth = Infinity;
            let maxDepth = -Infinity;
            for (const x of [-halfLength, halfLength]) {
              for (const z of [-halfWidth, halfWidth]) {
                for (const y of [0, 13]) {
                  const point = new court.ballGroup.position.constructor(x, y, z)
                    .applyMatrix4(inverse);
                  maxX = Math.max(maxX, Math.abs(point.x));
                  maxY = Math.max(maxY, Math.abs(point.y));
                  minDepth = Math.min(minDepth, -point.z);
                  maxDepth = Math.max(maxDepth, -point.z);
                }
              }
            }
            return { maxX, maxY, minDepth, maxDepth, near: shadow.near, far: shadow.far, top: shadow.top, right: shadow.right };
          })(),
          halfLength,
          canvasWidth: court.canvas.clientWidth,
          canvasHeight: court.canvas.clientHeight,
          labels: [...court.players.values()].filter((item) => item.label?.visible).length,
        };
      })()`);
      check("球员位置与帧数据一致", geometry.positionError !== null && geometry.positionError < 0.01,
        `误差=${geometry.positionError?.toFixed(4)}ft`);
      check(
        "全场四角在镜头内",
        geometry.corners.every(([x, y, z]) => Math.abs(x) <= 1.02 && Math.abs(y) <= 1.02 && z > -1 && z < 1),
        JSON.stringify(geometry.corners.map(([x, y]) => [x.toFixed(2), y.toFixed(2)])),
      );
      check("球在持球人手上", geometry.handDistance !== null && geometry.handDistance < 2.6,
        `距离=${geometry.handDistance?.toFixed(2)}ft`);
      check("渲染画布尺寸有效", geometry.canvasWidth > 400 && geometry.canvasHeight > 200,
        `${geometry.canvasWidth}x${geometry.canvasHeight}`);
      check("号码标签已挂载", geometry.labels === 10, `labels=${geometry.labels}`);
      check("小人朝向与 facing 一致", geometry.facingError === null || geometry.facingError < 0.001,
        `误差=${geometry.facingError === null ? "无朝向" : geometry.facingError.toFixed(4)}rad`);
      check("模型前向与资产骨骼对齐", geometry.assetAlignment !== null && geometry.assetAlignment < 0.02,
        `剩余偏差=${geometry.assetAlignment === null ? "无脚骨" : geometry.assetAlignment.toFixed(4)}rad`);
      check(
        "模型尺寸与落地高度合理",
        geometry.renderLowest !== null &&
          geometry.renderLowest > -0.4 &&
          geometry.renderLowest < 1.2 &&
          geometry.headHeight !== null &&
          geometry.headHeight > 4.8 &&
          geometry.headHeight < 7.4,
        `最低点=${geometry.renderLowest}ft 头高=${geometry.headHeight}ft`,
      );
      check(
        "背向镜头的球员显示背面",
        geometry.facingSample.away === 0 ||
          geometry.facingSample.backToCamera === geometry.facingSample.away,
        `背向人数=${geometry.facingSample.away} 显示背面=${geometry.facingSample.backToCamera}`,
      );
      check("转向平滑不超过一帧跳跃", geometry.facingSample.lag <= 1.85,
        `滞后=${geometry.facingSample.lag.toFixed(3)}rad`);
      check("人形在画面中有可读尺寸", geometry.figurePixels !== null && geometry.figurePixels >= 18 && geometry.figurePixels <= 90,
        `高度=${geometry.figurePixels?.toFixed(1)}px`);
      check("号码标签高于头部", (geometry.labelHeight ?? 0) > 7, `y=${geometry.labelHeight?.toFixed(2)}ft`);
      check("小人使用卡通渐层材质", geometry.materials.toon >= 40, `toon=${geometry.materials.toon}`);
      check("使用正交平行投影", geometry.orthographic === true);
      check("同一高度的球员尺寸一致", geometry.projectionSpread !== null && geometry.projectionSpread < 1.001,
        `最大/最小=${geometry.projectionSpread?.toFixed(4)}`);
      const verticalSpan = Math.abs(geometry.corners[0][1] - geometry.corners[2][1]);
      check("球场在画面中占据足够高度", verticalSpan > 0.6, `跨度=${verticalSpan.toFixed(2)} NDC`);
      const hoopsOk = geometry.hoops.length === 2 && geometry.hoops.every((hoop) =>
        Math.abs(hoop.poleX) > geometry.halfLength &&
        Math.abs(hoop.rimX) < geometry.halfLength &&
        Math.abs(hoop.rimY - 10) < 0.001 &&
        Math.abs(Math.abs(hoop.rimX - hoop.backboardX) - 1.25) < 0.05,
      );
      check("篮架几何放在底线两侧", hoopsOk,
        JSON.stringify(geometry.hoops.map((hoop) => [hoop.rimX.toFixed(2), hoop.backboardX.toFixed(2), hoop.poleX.toFixed(2)])));
      const shadow = geometry.shadowCoverage;
      check(
        "阴影相机覆盖全场",
        shadow !== null &&
          shadow.maxX <= Math.abs(shadow.right) &&
          shadow.maxY <= Math.abs(shadow.top) &&
          shadow.minDepth >= shadow.near &&
          shadow.maxDepth <= shadow.far,
        shadow
          ? `横向=${shadow.maxX.toFixed(1)}/${shadow.right} 纵向=${shadow.maxY.toFixed(1)}/${shadow.top} 深度=${shadow.minDepth.toFixed(0)}..${shadow.maxDepth.toFixed(0)}`
          : "无阴影相机",
      );

      await devtools.evaluate(`window.__nbaCourtView.cycleCamera()`);
      await new Promise((resolve) => setTimeout(resolve, 3000));
      const follow = await devtools.evaluate(`(() => {
        const court = window.__nbaDebug.court3d;
        const columns = [...court.players.values()]
          .filter((item) => item.root.visible)
          .map((item) => {
            const base = court.ballGroup.position
              .clone()
              .set(item.root.position.x, 0, item.root.position.z)
              .project(court.camera);
            const top = court.ballGroup.position
              .clone()
              .set(item.root.position.x, 7, item.root.position.z)
              .project(court.camera);
            return Math.abs(base.y - top.y) * court.canvas.clientHeight * 0.5;
          })
          .sort((a, b) => a - b);
        return {
          preset: court.preset,
          frustum: (court.camera.top - court.camera.bottom) / court.camera.zoom,
          figurePixels: columns.length ? columns[Math.floor(columns.length / 2)] : null,
          label: document.getElementById("cameraModeLabel").textContent,
        };
      })()`);
      check("切到跟拍镜头", follow.preset === "follow" && follow.label === "跟拍镜头", JSON.stringify(follow));
      check("跟拍镜头收紧视野", follow.frustum < geometry.frustum * 0.75,
        `${geometry.frustum.toFixed(1)}ft → ${follow.frustum.toFixed(1)}ft`);
      check("跟拍下人物更大", follow.figurePixels > geometry.figurePixels * 1.5,
        `${geometry.figurePixels?.toFixed(1)}px → ${follow.figurePixels.toFixed(1)}px`);

      const cycleBack = await devtools.evaluate(`(() => {
        window.__nbaCourtView.cycleCamera();
        window.__nbaCourtView.cycleCamera();
        return { preset: window.__nbaDebug.court3d.preset, label: document.getElementById("cameraModeLabel").textContent };
      })()`);
      check("镜头循环回全场", cycleBack.preset === "broadcast" && cycleBack.label === "全场镜头", JSON.stringify(cycleBack));

      const panControl = await devtools.evaluate(`(() => {
        const court = window.__nbaDebug.court3d;
        const before = court.camera.position.clone();
        court.controls.target.set(400, 1.2, 400);
        court.clampPan();
        const target = court.controls.target.clone();
        const cameraMoved = court.camera.position.distanceTo(before);
        return {
          rotate: court.controls.enableRotate,
          target: [target.x, target.z],
          limits: [court.halfLength - 12, court.halfWidth - 6],
          cameraMoved,
        };
      })()`);
      check("镜头不允许旋转", panControl.rotate === false);
      check(
        "平移被限制在球场范围",
        Math.abs(panControl.target[0]) <= panControl.limits[0] + 0.001 &&
          Math.abs(panControl.target[1]) <= panControl.limits[1] + 0.001,
        JSON.stringify(panControl.target.map((value) => value.toFixed(1))),
      );
      check("平移限制保持视角不变", panControl.cameraMoved > 100, `相机同步移动=${panControl.cameraMoved.toFixed(1)}ft`);
      await devtools.evaluate(`window.__nbaDebug.court3d.applyPreset("broadcast")`);

      const gestureStart = await devtools.evaluate(`(() => {
        const court = window.__nbaDebug.court3d;
        const view = [...court.players.values()][0];
        view.playGesture("shot");
        return { during: view.current?.getClip().name ?? null };
      })()`);
      await new Promise((resolve) => setTimeout(resolve, 3500));
      const gestureEnd = await devtools.evaluate(`(() => {
        const court = window.__nbaDebug.court3d;
        const view = [...court.players.values()][0];
        return { after: view.current?.getClip().name ?? null, gesture: view.gesture ?? null };
      })()`);
      check("投篮手势切换到出手片段", /Man_Punch$/.test(gestureStart.during ?? ""), `clip=${gestureStart.during}`);
      check("手势结束后回到持续状态", /(Man_Idle|Man_Walk|Man_Run)$/.test(gestureEnd.after ?? "") && gestureEnd.gesture === null,
        `clip=${gestureEnd.after}`);

      const toggle2d = await devtools.evaluate(`(() => {
        window.__nbaCourtView.setMode("2d");
        const stage = document.getElementById("courtStage");
        return {
          mode: window.__nbaDebug.viewMode,
          classed: stage.classList.contains("view-3d"),
          canvasVisible: document.getElementById("court3dCanvas").style.display,
          hitPlayers: window.__nbaDebug.hitPlayers.length,
        };
      })()`);
      check("切换二维战术板", toggle2d.mode === "2d" && toggle2d.classed === false, JSON.stringify(toggle2d));
      check("二维视图重新绘制命中锚点", toggle2d.hitPlayers === 10, `hitPlayers=${toggle2d.hitPlayers}`);

      const toggle3d = await devtools.evaluate(`(() => {
        window.__nbaCourtView.setMode("3d");
        return {
          mode: window.__nbaDebug.viewMode,
          classed: document.getElementById("courtStage").classList.contains("view-3d"),
        };
      })()`);
      check("切回 2.5D 球场", toggle3d.mode === "3d" && toggle3d.classed === true, JSON.stringify(toggle3d));

      const before = await devtools.evaluate(`window.__nbaDebug.court3d.frames`);
      await new Promise((resolve) => setTimeout(resolve, 1200));
      const after = await devtools.evaluate(`window.__nbaDebug.court3d.frames`);
      check("渲染持续运行", after > before, `${before} → ${after}`);

      const shot = await devtools.send("Page.captureScreenshot", { format: "png" });
      const bytes = Buffer.from(shot.data, "base64").length;
      check("截图非空", bytes > 20000, `${Math.round(bytes / 1024)} KB`);
    }

    check("没有未捕获异常", devtools.exceptions.length === 0, devtools.exceptions.slice(0, 3).join(" | "));
    const actionableErrors = devtools.consoleErrors.filter(
      (line) => !/favicon|net::ERR_|Failed to load resource/.test(line),
    );
    check("控制台没有错误", actionableErrors.length === 0, actionableErrors.slice(0, 3).join(" | "));
  } finally {
    devtools?.close();
    chrome.kill("SIGKILL");
    rmSync(PROFILE_DIR, { recursive: true, force: true });
  }

  if (failures.length) {
    console.log(`\n失败项：${failures.join("、")}`);
    process.exit(1);
  }
  console.log("\n2.5D 球场视图验证通过");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
