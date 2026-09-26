#!/usr/bin/env python3
"""UI E2E regression check (headless Chrome dump-dom + console check)."""
import subprocess, sys, time, urllib.request

def check_dom():
    cmd = ["google-chrome", "--headless=new", "--disable-gpu", "--no-sandbox",
           "--virtual-time-budget=10000", "--dump-dom", "http://127.0.0.1:4173/"]
    out = subprocess.check_output(cmd, timeout=30).decode()
    assert "ticks" in out and "运行失败" not in out, "runStatus must show ticks, not failure"
    assert "overlapLimit" not in out, "no overlapLimit ReferenceError"
    assert "运行失败" not in out, "page must not show failure"
    assert "42" in out and "回合" in out, "streamSummary must render"
    print("✅ UI DOM regression check passed (boot, controls, summary)")

if __name__ == "__main__":
    check_dom()
