#!/usr/bin/env python3
"""CI 变更基线：最近一次绿色 commit 与逐 commit 审计范围。

## 为什么需要这个模块

变更检测若只比较 `HEAD~1..HEAD`，会产生「洗白」漏洞：
commit A 携带大量行为改动导致 CI 失败后，紧随其后的 commit B
哪怕只改一个标点，也会被判定为「无相关改动」，跳过 clippy 与
整条测试链，并且部署阶段的 success-or-skipped 条件直接放行部署。
被破坏的分支从未被验证就能进入生产。

正确基线是「当前分支上最近一次 CI 全绿的 commit」：
绿色之后的一切改动（无论分装成几个 commit）都会进入本次 diff，
失败未修复就不存在新的绿色基线，跳过路径被彻底关闭。

## 对外接口

- `green_baseline() -> str | None`
    返回当前分支最近一次全绿 commit 的 SHA；找不到（新分支、
    历史被改写、或全绿 commit 不在当前祖先链上）返回 None，
    调用方必须把 None 视为全量处理。
- `audit_range(rev_range_arg: str | None) -> list[str]`
    返回待审计的 commit SHA 列表（旧→新）。显式给出 `A..B`
    时审计该区间；否则审计绿色基线（不含）到 HEAD 之间的
    commit；没有绿色基线时只审计 HEAD 自身。

调用方（GitHub Actions 与本地守卫）共享本模块，保证
「job 级路径过滤」与「commit 级守卫审计」看到同一个基线。
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GH = shutil.which("gh")

# 缓存上限：workflow 运行数太多时只回看最近的一部分，
# 足够覆盖本分支连续失败重试的窗口。
_MAX_RUNS = 100


def _git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout


def _branch() -> str:
    head = _git("branch", "--show-current").strip()
    if head:
        return head
    # detached HEAD（workflow 里 checkout pull request 时的形态）：
    # GitHub 的分支名暴露在环境变量里。
    return os.environ.get("GITHUB_HEAD_REF", "").strip()


def _gh_run_list(branch: str) -> str:
    if not GH:
        raise FileNotFoundError("gh cli not available")
    return subprocess.run(
        [GH, "run", "list", "--branch", branch, "--event", "push",
         "--limit", str(_MAX_RUNS), "--json", "headSha,conclusion"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout


def green_baseline() -> str | None:
    """当前分支最近一次 CI 全绿 commit 的 SHA；找不到返回 None。

    判定标准：该 commit 是 `push` 事件触发的 workflow run 的
    headSha，且 conclusion == success。只接受属于当前 HEAD
    祖先链的 commit，防止 rebase 后旧分支的绿色 commit 误当基线。
    """
    branch = _branch()
    try:
        runs = _gh_run_list(branch)
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None
    try:
        entries = json.loads(runs)
    except json.JSONDecodeError:
        return None
    for entry in entries:
        if entry.get("conclusion") != "success":
            continue
        sha = entry.get("headSha", "")
        if not sha:
            continue
        try:
            # `git merge-base --is-ancestor A HEAD` 以退出码表达结果。
            subprocess.run(
                ["git", "merge-base", "--is-ancestor", sha, "HEAD"],
                cwd=ROOT, capture_output=True, check=True,
            )
        except subprocess.CalledProcessError:
            continue
        return sha
    return None


def commits_between(base: str, head: str) -> list[str]:
    """base（不含）到 head（含）之间的 commit SHA，旧→新。"""
    out = _git("rev-list", "--reverse", f"{base}..{head}")
    return [line for line in out.splitlines() if line.strip()]


def audit_range(rev_range_arg: str | None) -> list[str]:
    """返回待审计的 commit SHA 列表（旧→新）。

    显式 `A..B` → 该区间；无绿色基线 → 仅 HEAD；
    否则绿色基线之后的所有 commit。
    """
    if rev_range_arg:
        base, _, tip = rev_range_arg.partition("..")
        tip = tip or "HEAD"
        return commits_between(base, tip)
    baseline = green_baseline()
    if baseline is None:
        return _git("rev-list", "--reverse", "-n", "1", "HEAD").split()
    return commits_between(baseline, "HEAD")


def changed_files(rev_range_arg: str | None) -> list[str]:
    """待审计范围内全部改动的文件路径（相对仓库根，已去重）。

    没有绿色基线时按全量处理：单 commit 的 diff 树等价于「整个
    工作区都算改动」，对应 workflow 的全量测试分支。
    """
    baseline = green_baseline()
    if rev_range_arg:
        out = _git("diff", "--name-only", rev_range_arg)
    elif baseline is None:
        out = _git("ls-files")
    else:
        out = _git("diff", "--name-only", f"{baseline}..HEAD")
    seen: list[str] = []
    for line in out.splitlines():
        if line.strip() and line not in seen:
            seen.append(line)
    return seen


def self_test() -> int:
    """负面对照：基线解析与审计范围在当前仓库上必须自洽。"""
    print("🧪 ci-baseline self-test")
    ok = True

    baseline = green_baseline()
    print(f"   绿色基线: {baseline[:12] if baseline else '（无，按全量处理）'}")
    commits = audit_range(None)
    if not commits:
        # 当前 HEAD 本身就是绿色基线时，待审计列表为空是正确状态：
        # 绿色之后没有新改动，路径检测应当输出「无改动」。
        print("   ✅ 绿色基线即 HEAD，无待审计改动（正确的空状态）")
    else:
        print(f"   审计范围: {len(commits)} 个 commit，末尾 {commits[-1][:12]}")

    # 审计范围内的改动必须能在「基线..HEAD」diff 中找到对应文件。
    head = _git("rev-parse", "HEAD").strip()
    if baseline is not None:
        files = changed_files(None)
        diff_files = {
            line.strip()
            for line in _git("diff", "--name-only", f"{baseline}..{head}").splitlines()
            if line.strip()
        }
        if not diff_files:
            if files:
                print("   ❌ 基线即 HEAD 时 changed_files 应为空")
                ok = False
            else:
                print("   ✅ 基线即 HEAD，changed_files 为空（一致）")
        elif set(files) == diff_files:
            print(f"   ✅ changed_files 与 diff 一致（{len(files)} 个文件）")
        else:
            print("   ❌ changed_files 与基线 diff 不一致")
            ok = False

    # 基线若存在，必须是 HEAD 的祖先且不是 HEAD 自身之后的东西。
    if baseline is not None:
        if baseline in commits:
            print("   ❌ 绿色基线不应出现在待审计列表里")
            ok = False
        else:
            print("   ✅ 绿色基线在审计范围之外（祖先位置正确）")

    print("   self-test:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    import argparse

    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", action="store_true",
                    help="只打印绿色基线 SHA（无则空输出），供 CI 脚本取用")
    ap.add_argument("--range", dest="rev_range", default=None,
                    help="git diff 范围，例如 HEAD~1..HEAD")
    ap.add_argument("--files", action="store_true",
                    help="打印待审计范围内改动的文件路径")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if args.baseline:
        baseline = green_baseline()
        if baseline:
            print(baseline)
        return 0

    if args.files:
        for f in changed_files(args.rev_range):
            print(f)
        return 0

    commits = audit_range(args.rev_range)
    baseline = green_baseline()
    print(f"基线: {baseline or '（无，按全量处理）'}")
    print(f"审计 {len(commits)} 个 commit")
    for c in commits:
        print(c)
    return 0


if __name__ == "__main__":
    sys.exit(main())
