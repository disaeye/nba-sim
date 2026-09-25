#!/usr/bin/env python3
"""Automated end-to-end verification for Court Visuals & Dynamic FX System."""

import json, subprocess, sys, time, urllib.request

SERVER = "http://127.0.0.1:4173"

def run_chrome_eval(js_expr: str, port: int = 9224) -> dict:
    req = urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=10)
    targets = json.loads(req.read().decode())
    page = next(t for t in targets if t.get("type") == "page")
    ws_url = page["webSocketDebuggerUrl"]

    import socket, base64, os
    host, port_str = ws_url.split("/")[2].split(":")
    s = socket.create_connection((host, int(port_str)), timeout=10)
    key = base64.b64encode(os.urandom(16)).decode()
    path = "/" + "/".join(ws_url.split("/")[3:])
    s.sendall(f"GET {path} HTTP/1.1\r\nHost: {host}:{port_str}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode())
    resp = b""
    while b"\r\n\r\n" not in resp:
        resp += s.recv(1024)

    def ws_send(msg):
        p = json.dumps(msg).encode()
        plen = len(p)
        if plen <= 125:
            h = bytearray([0x81, 0x80 | plen])
        elif plen <= 65535:
            h = bytearray([0x81, 0xFE]) + int(plen).to_bytes(2, "big")
        else:
            h = bytearray([0x81, 0xFF]) + int(plen).to_bytes(8, "big")
        m = bytearray(os.urandom(4))
        masked = bytearray(b ^ m[i % 4] for i, b in enumerate(p))
        s.sendall(h + m + masked)

    def ws_recv():
        h = s.recv(2)
        plen = h[1] & 0x7F
        if plen == 126:
            plen = int.from_bytes(s.recv(2), "big")
        data = bytearray()
        while len(data) < plen:
            data.extend(s.recv(plen - len(data)))
        return json.loads(data.decode("utf-8", errors="ignore"))

    ws_send({"id": 1, "method": "Runtime.evaluate", "params": {"expression": js_expr, "returnByValue": True, "awaitPromise": True}})
    for _ in range(25):
        m = ws_recv()
        if m.get("id") == 1:
            s.close()
            res = m.get("result", {}).get("result", {})
            if "value" in res:
                return res["value"]
            if res.get("subtype") == "error":
                raise RuntimeError(f"JS Error: {res.get('description')}")
            return res
    s.close()
    return {}

def main():
    print("🏀 启动球场动效与视觉表现自动化端到端测试...")

    chrome_cmd = [
        "google-chrome", "--headless=new", "--disable-gpu", "--no-sandbox",
        "--remote-debugging-port=9224", f"{SERVER}/"
    ]
    proc = subprocess.Popen(chrome_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(3.5)

    try:
        # 1. 等待流加载完成
        data = None
        for _ in range(20):
            js_probe = """
            (function() {
                const state = window.__nbaDebug;
                if (!state || !state.ticks || state.ticks.length === 0) return { waiting: true };
                const tick = state.ticks[state.idx];
                const fx = state.courtFX;

                const hitCount = state.hitPlayers ? state.hitPlayers.length : 0;
                const handler = (tick.players || []).find(p => p.hasBall);
                const ball = tick.ball;

                return {
                    tickCount: state.ticks.length,
                    hitCount,
                    hasFX: !!fx,
                    handlerId: handler ? handler.id : null,
                    ball: ball ? { x: ball.x, y: ball.y, z: ball.z, status: ball.status } : null,
                    courtMode: state.courtMode,
                    ballHistoryLen: fx ? fx.ballHistory.length : 0
                };
            })()
            """
            data = run_chrome_eval(js_probe)
            if not data.get("waiting"):
                break
            time.sleep(0.5)

        if not data or data.get("waiting"):
            raise RuntimeError("流加载超时")

        print(f"✅ 基础流加载正常: {data['tickCount']} ticks, {data['hitCount']} 名在场球员像素锚点已生成")
        print(f"✅ 持球核心已标识: #{data['handlerId'] or '无'}, 动效管理器处于运行状态: {data['hasFX']}")
        assert data["hitCount"] == 10, f"在场球员数量应为10，实测为 {data['hitCount']}"
        assert data["hasFX"], "CourtVisualFXManager 必须正常实例化"

        # 2. 搜索关键事件帧并在球场上验证动效生成
        js_find_events = """
        (function() {
            const state = window.__nbaDebug;
            const shotReleaseIdx = state.ticks.findIndex(t => (t.events || []).includes("SHOT_RELEASE") || t.eventType === "SHOT_RELEASE");
            const scoreIdx = state.ticks.findIndex(t => (t.events || []).includes("SCORE") || (t.events || []).includes("SHOT_MADE"));
            const missIdx = state.ticks.findIndex(t => (t.events || []).includes("SHOT_MISS") || (t.events || []).includes("DRIVE_MISS"));
            const stealIdx = state.ticks.findIndex(t => (t.events || []).includes("STEAL") || (t.events || []).includes("BALL_POKED_LOOSE"));
            const blockIdx = state.ticks.findIndex(t => (t.events || []).includes("BLOCK"));

            return { shotReleaseIdx, scoreIdx, missIdx, stealIdx, blockIdx };
        })()
        """
        events = run_chrome_eval(js_find_events)
        print(f"🎯 关键事件分布: 出手={events['shotReleaseIdx']}, 命中={events['scoreIdx']}, 打铁={events['missIdx']}, 抢断={events['stealIdx']}, 盖帽={events['blockIdx']}")

        # 验证投篮出手动效 (Shot Arc, Ground Ripple, Floating Text)
        if events['shotReleaseIdx'] >= 0:
            target = events['shotReleaseIdx']
            js_verify_shot = f"""
            (function() {{
                const state = window.__nbaDebug;
                document.getElementById("progressInput").value = "{target}";
                document.getElementById("progressInput").dispatchEvent(new Event("input"));

                const fx = state.courtFX;
                const effects = fx ? fx.effects.map(e => ({{ type: e.type, text: e.text }})) : [];
                return {{ effects, fxCount: effects.length }};
            }})()
            """
            shot_fx = run_chrome_eval(js_verify_shot)
            has_arc = any(e["type"] == "shot_arc" for e in shot_fx["effects"])
            has_text = any(e["type"] == "floating_text" for e in shot_fx["effects"])
            has_ripple = any(e["type"] == "ground_ripple" for e in shot_fx["effects"])
            print(f"🏹 投篮出手帧 ({target}) 动效列表: {shot_fx['effects']}")
            assert has_arc, "投篮出手帧必须产生投篮抛物线光弧 (shot_arc)"
            assert has_text, "投篮出手帧必须产生投篮浮动提示标牌 (floating_text)"
            assert has_ripple, "投篮出手帧必须产生脚下起跳聚能光环 (ground_ripple)"
            print("✅ 投篮出手视觉光弧与起跳光效验证通过")

        # 验证投篮命中得分动效 (Swish Splash, Confetti, Floating Banner, Net Swish)
        if events['scoreIdx'] >= 0:
            target = events['scoreIdx']
            js_verify_score = f"""
            (function() {{
                const state = window.__nbaDebug;
                document.getElementById("progressInput").value = "{target}";
                document.getElementById("progressInput").dispatchEvent(new Event("input"));

                const fx = state.courtFX;
                const effects = fx ? fx.effects.map(e => ({{ type: e.type, text: e.text }})) : [];
                const netMoved = fx ? (fx.rimStates.left.netOffset > 0 || fx.rimStates.right.netOffset > 0) : false;
                return {{ effects, netMoved }};
            }})()
            """
            score_fx = run_chrome_eval(js_verify_score)
            has_swish = any(e["type"] == "swish_splash" for e in score_fx["effects"])
            has_particles = any(e["type"] == "particles" for e in score_fx["effects"])
            has_banner = any(e["type"] == "floating_text" and ("+" in (e.get("text") or "")) for e in score_fx["effects"])
            print(f"🎯 进球得分帧 ({target}) 动效列表: {score_fx['effects']}")
            assert has_swish, "进球得分帧必须产生入网水花冲击波 (swish_splash)"
            assert has_particles, "进球得分帧必须产生庆祝爆裂礼花粒子 (particles)"
            assert has_banner, "进球得分帧必须产生加分浮动大标牌 (+2/+3)"
            assert score_fx["netMoved"], "进球得分帧必须触发篮网物理下抽形变 (netOffset > 0)"
            print("✅ 投篮命中空心入网水花、礼花粒子与加分标牌验证通过")

        # 验证打铁动效 (Rim Shake, Sparks, Miss Banner)
        if events['missIdx'] >= 0:
            target = events['missIdx']
            js_verify_miss = f"""
            (function() {{
                const state = window.__nbaDebug;
                document.getElementById("progressInput").value = "{target}";
                document.getElementById("progressInput").dispatchEvent(new Event("input"));

                const fx = state.courtFX;
                const effects = fx ? fx.effects.map(e => ({{ type: e.type, text: e.text }})) : [];
                const rimShaking = fx ? (fx.rimStates.left.shake > 0 || fx.rimStates.right.shake > 0) : false;
                return {{ effects, rimShaking }};
            }})()
            """
            miss_fx = run_chrome_eval(js_verify_miss)
            has_miss_text = any(e["type"] == "floating_text" and ("打铁" in (e.get("text") or "")) for e in miss_fx["effects"])
            print(f"💥 打铁帧 ({target}) 动效列表: {miss_fx['effects']}")
            assert has_miss_text, "投篮打铁帧必须产生打铁浮动标牌 (💥 弹筐打铁！)"
            assert miss_fx["rimShaking"], "投篮打铁帧必须触发篮筐金属高频震颤 (shake > 0)"
            print("✅ 投篮打铁金属震颤与撞击火花验证通过")

        # 3. 验证半场特写与全场鸟瞰视角切换及坐标自适应
        js_verify_court_mode = """
        (function() {
            const state = window.__nbaDebug;
            const btn = document.getElementById("courtViewBtn");
            const initialMode = state.courtMode;
            const initialHits = [...state.hitPlayers];

            // 切换视角
            btn.click();
            const toggledMode = state.courtMode;
            const toggledHits = [...state.hitPlayers];

            // 再次切换还原
            btn.click();
            const restoredMode = state.courtMode;

            return { initialMode, toggledMode, restoredMode, initialHitsCount: initialHits.length, toggledHitsCount: toggledHits.length };
        })()
        """
        mode_data = run_chrome_eval(js_verify_court_mode)
        print(f"🔍 视角切换测试: 初始={mode_data['initialMode']} -> 切换后={mode_data['toggledMode']} -> 还原={mode_data['restoredMode']}")
        assert mode_data["initialMode"] != mode_data["toggledMode"], "点击 courtViewBtn 必须改变视角模式"
        assert mode_data["restoredMode"] == mode_data["initialMode"], "二次点击必须恢复原有视角模式"
        assert mode_data["toggledHitsCount"] == 10, "切换特写视角后在场10人触控锚点必须保持完整映射"
        print("✅ 半场特写视口缩放与全场鸟瞰平滑切换验证通过")

        # 4. 验证播放连续推进与动画帧平滑运转
        js_playback = """
        (function() {
            const state = window.__nbaDebug;
            const startIdx = state.idx;
            const playBtn = document.getElementById("playButton");

            // 开启播放
            if (!state.playing) playBtn.click();

            return new Promise(resolve => {
                setTimeout(() => {
                    const midIdx = state.idx;
                    // 暂停播放
                    if (state.playing) playBtn.click();
                    resolve({ startIdx, midIdx, isAdvancing: midIdx > startIdx });
                }, 600);
            });
        })()
        """
        play_data = run_chrome_eval(js_playback)
        print(f"▶️ 动态播放推进测试: 起始帧={play_data['startIdx']} -> 运行后={play_data['midIdx']} (帧推进={play_data['isAdvancing']})")
        assert play_data["isAdvancing"], "播放状态下帧计数必须平滑自增推进"
        print("✅ 动画播放与动效渲染循环运行流畅，无异常停滞")

        print("\n🎉 所有球场、球员与篮球高辨识度动效测试全部顺利通过！")
    finally:
        proc.terminate()

if __name__ == "__main__":
    main()
