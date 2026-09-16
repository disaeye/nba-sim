#!/usr/bin/env python3
"""watch_ci.py - 本地实时监听 GitHub Actions 流水线进度与 Vercel 部署状态"""

import subprocess
import sys
import time
import urllib.request
import json

def get_current_commit():
    res = subprocess.run(["git", "rev-parse", "HEAD"], stdout=subprocess.PIPE, text=True, check=True)
    return res.stdout.strip()

def main():
    sha = get_current_commit()
    print(f"📡 正在追踪当前提交的云端 CI/CD 状态: {sha[:7]}")
    print("👉 网页端实时看板: https://github.com/disaeye/nba-sim/actions")
    print("👉 线上预览地址:   https://nba-sim-eight.vercel.app\n")
    print("💡 提示：按 Ctrl+C 可随时退出监听，云端部署不受影响。\n")

if __name__ == "__main__":
    main()
