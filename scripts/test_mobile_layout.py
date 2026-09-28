import base64
import json
import os
import socket
import subprocess
import time
import urllib.request
from typing import Any


def _recv_frame(sock: socket.socket) -> dict[str, Any] | None:
    """读取一条 WebSocket 帧并解析为 JSON；连接关闭时返回 None。"""
    hdr = sock.recv(2)
    if not hdr:
        return None
    length = hdr[1] & 0x7F
    if length == 126:
        length = int.from_bytes(sock.recv(2), "big")
    raw = bytearray()
    while len(raw) < length:
        chunk = sock.recv(length - len(raw))
        if not chunk:
            return None
        raw.extend(chunk)
    return json.loads(raw.decode("utf-8", errors="ignore"))


def eval_js(js: str, width: int = 390, height: int = 844) -> dict[str, Any]:
    """连接调试端口，模拟移动端视口并执行 JavaScript，返回结果值。"""
    req = urllib.request.urlopen("http://127.0.0.1:9225/json/list", timeout=10)
    targets = json.loads(req.read().decode())
    page = next(t for t in targets if t.get("type") == "page")
    ws_url = page["webSocketDebuggerUrl"]

    host, port_str = ws_url.split("/")[2].split(":")
    sock = socket.create_connection((host, int(port_str)), timeout=10)
    key = base64.b64encode(os.urandom(16)).decode()
    path = "/" + "/".join(ws_url.split("/")[3:])
    sock.sendall(
        f"GET {path} HTTP/1.1\r\nHost: {host}:{port_str}\r\n"
        "Upgrade: websocket\r\nConnection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode()
    )
    resp = b""
    while b"\r\n\r\n" not in resp:
        resp += sock.recv(1024)

    def send_cmd(cmd_id: int, method: str, params: dict[str, Any] | None = None) -> None:
        payload = {"id": cmd_id, "method": method}
        if params:
            payload["params"] = params
        data = json.dumps(payload).encode()
        size = len(data)
        if size <= 125:
            header = bytearray([0x81, 0x80 | size])
        else:
            header = bytearray([0x81, 0xFE]) + size.to_bytes(2, "big")
        mask = bytearray(os.urandom(4))
        masked = bytearray(b ^ mask[i % 4] for i, b in enumerate(data))
        sock.sendall(header + mask + masked)

    # 模拟移动端视口
    send_cmd(10, "Emulation.setDeviceMetricsOverride", {
        "width": width,
        "height": height,
        "deviceScaleFactor": 3,
        "mobile": True,
    })
    # 等待页面 DOM 就绪
    wait_js = (
        "Boolean(document.querySelector('.app-nav') && "
        "document.querySelector('.match-scoreboard'))"
    )
    for _ in range(20):
        send_cmd(99, "Runtime.evaluate", {"expression": wait_js, "returnByValue": True})
        ready = False
        for _ in range(10):
            msg = _recv_frame(sock)
            if msg is None:
                break
            if msg.get("id") == 99:
                ready = msg.get("result", {}).get("result", {}).get("value", False)
                break
        if ready:
            break
        time.sleep(0.3)

    send_cmd(1, "Runtime.evaluate", {"expression": js, "returnByValue": True})

    value: Any = None
    for _ in range(25):
        msg = _recv_frame(sock)
        if msg is None:
            break
        if msg.get("id") == 1:
            value = msg.get("result", {}).get("result", {}).get("value")
            break
    sock.close()
    if not isinstance(value, dict):
        raise RuntimeError("CDP Runtime.evaluate 未返回结果对象")
    return value


chrome_cmd = [
    "google-chrome",
    "--headless=new",
    "--disable-gpu",
    "--no-sandbox",
    "--remote-debugging-port=9225",
    "http://127.0.0.1:4173/",
]
proc = subprocess.Popen(chrome_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(3.5)
try:
    js_check = """
    (function() {
        const bodyWidth = document.body.clientWidth;
        const scrollWidth = document.documentElement.scrollWidth;
        const windowWidth = window.innerWidth;
        const hasHorizontalOverflow = scrollWidth > windowWidth;

        function getBox(selector) {
            const el = document.querySelector(selector);
            if (!el) return null;
            const r = el.getBoundingClientRect();
            return { width: Math.round(r.width), height: Math.round(r.height), top: Math.round(r.top), left: Math.round(r.left) };
        }

        // 检查顶栏子元素分布
        const navBrand = getBox('.nav-brand-section');
        const navCtrl = getBox('.nav-control-section');
        const navRight = getBox('.nav-right-section');

        // 检查记分牌子元素
        const awayTeam = getBox('.away-team-block');
        const homeTeam = getBox('.home-team-block');
        const scoreCenter = getBox('.score-center-cluster');

        // 检查 HUD 按钮栏
        const hudToolbar = getBox('.court-hud-toolbar');

        // 检查底部 6 个 tab 按钮的宽度
        const tabs = Array.from(document.querySelectorAll('.segmented-nav .tab-button')).map(btn => {
            const r = btn.getBoundingClientRect();
            return { text: btn.innerText.trim(), width: Math.round(r.width), left: Math.round(r.left) };
        });

        // 检查播放控制台子区域
        const transportBtns = getBox('.transport-btns');
        const speedGroup = getBox('.speed-group');
        const jumpGroup = getBox('.jump-frame-group');

        return {
            windowWidth,
            bodyWidth,
            scrollWidth,
            hasHorizontalOverflow,
            nav: getBox('.app-nav'),
            navBrand,
            navCtrl,
            navRight,
            scoreboard: getBox('.match-scoreboard'),
            awayTeam,
            homeTeam,
            scoreCenter,
            courtWrap: getBox('.court-stage-wrap'),
            hudToolbar,
            transportBtns,
            speedGroup,
            jumpGroup,
            playBtn: getBox('#playButton'),
            segmentedNav: getBox('.segmented-nav'),
            tabs
        };
    })()
    """
    res_375 = eval_js(js_check, width=375, height=812)
    print("📱 375px (iPhone SE / mini) 渲染指标:", json.dumps(res_375, indent=2))
    assert not res_375["hasHorizontalOverflow"], "375px 屏幕严禁产生水平滚动条溢出"
    assert res_375["nav"]["height"] <= 95, f"顶栏高度超出移动端标准: {res_375['nav']['height']}px > 95px"
    assert res_375["scoreboard"]["height"] <= 90, f"记分牌高度过高: {res_375['scoreboard']['height']}px > 90px"
    assert res_375["hudToolbar"] is None or res_375["hudToolbar"]["width"] == 0, "移动端必须隐藏悬浮 HUD 工具栏释放纯净球场"
    assert res_375["playBtn"]["width"] >= 44 and res_375["playBtn"]["height"] >= 44, "播放按钮触控区需符合移动端 >=44px 标准"

    res_390 = eval_js(js_check, width=390, height=844)
    print("📱 390px (iPhone 14 / Pro) 渲染指标:", json.dumps(res_390, indent=2))
    assert not res_390["hasHorizontalOverflow"], "390px 屏幕严禁产生水平滚动条溢出"
    assert res_390["nav"]["height"] <= 95, f"顶栏高度超出移动端标准: {res_390['nav']['height']}px > 95px"
    assert res_390["scoreboard"]["height"] <= 90, f"记分牌高度过高: {res_390['scoreboard']['height']}px > 90px"
    assert res_390["hudToolbar"] is None or res_390["hudToolbar"]["width"] == 0, "移动端必须隐藏悬浮 HUD 工具栏释放纯净球场"
    assert res_390["playBtn"]["width"] >= 44 and res_390["playBtn"]["height"] >= 44, "播放按钮触控区需符合移动端 >=44px 标准"

    print("🎉 移动端 (375px & 390px) 布局与触控自适应全部检验合格！")
finally:
    proc.terminate()
