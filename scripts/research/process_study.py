import argparse
import csv
import json
import math
import random
import statistics
from collections import defaultdict
from pathlib import Path

SHOT_EVENTS = {1, 2}
REBOUND_EVENT = 4
TURNOVER_EVENT = 5
SHOT_ZONES = ("rim", "near", "mid", "three")


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pbp", required=True)
    parser.add_argument("--shotdetail", required=True)
    parser.add_argument("--sim", required=True)
    parser.add_argument("--reference", default="data/reference/stat_deltas.v1.json")
    parser.add_argument("--out", required=True)
    parser.add_argument("--bootstrap", type=int, default=2000)
    parser.add_argument("--seed", type=int, default=20261102)
    return parser.parse_args()


def read_text(path):
    try:
        return Path(path).read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"cannot read {path}: {error}") from error


def open_csv(path):
    try:
        return open(path, newline="", encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"cannot read {path}: {error}") from error


def read_json(path):
    text = read_text(path)
    try:
        return json.loads(text)
    except json.JSONDecodeError as error:
        raise SystemExit(f"cannot parse {path}: {error}") from error


def parse_clock(text):
    minutes, seconds = text.split(":")
    return int(minutes) * 60 + int(seconds)


def load_shot_zones(path):
    zones = {}
    with open_csv(path) as handle:
        for row in csv.DictReader(handle):
            game = row["GAME_ID"]
            event = row["GAME_EVENT_ID"]
            zones[(game, event)] = classify_zone(row)
    return zones


def classify_zone(row):
    basic = row["SHOT_ZONE_BASIC"]
    distance = float(row["SHOT_DISTANCE"])
    if basic in ("Restricted Area", "In The Paint (Non-RA)"):
        return "rim" if distance <= 4.0 else "near"
    if basic in ("Mid-Range",):
        return "mid"
    if basic in ("Left Corner 3", "Right Corner 3", "Above the Break 3", "Backcourt"):
        return "three"
    return "three" if distance >= 23.75 else "mid"


def real_events(path):
    games = defaultdict(list)
    with open_csv(path) as handle:
        for row in csv.DictReader(handle):
            games[row["GAME_ID"]].append(row)
    for events in games.values():
        events.sort(key=lambda row: int(row["EVENTNUM"]))
    return games


def real_team(row):
    value = row.get("PLAYER1_TEAM_ID") or ""
    return value.strip()


def real_process(games, shot_zones):
    samples = {
        "DREB": [],
        "OREB": [],
        "STEAL": [],
    }
    own_shot = {
        "DREB": [0, 0],
        "OREB": [0, 0],
        "STEAL": [0, 0],
    }
    oreb_zones = defaultdict(int)
    for game_id, events in games.items():
        team_ids = {
            real_team(event)
            for event in events
            if real_team(event) and int(event["EVENTMSGTYPE"]) in SHOT_EVENTS
        }
        last_shot_team = None
        for index, event in enumerate(events):
            event_type = int(event["EVENTMSGTYPE"])
            if event_type in SHOT_EVENTS:
                last_shot_team = real_team(event)
                continue
            kind = None
            team = None
            if event_type == REBOUND_EVENT:
                team = real_team(event)
                if not team:
                    continue
                if last_shot_team == team:
                    kind = "OREB"
                elif last_shot_team:
                    kind = "DREB"
            elif event_type == TURNOVER_EVENT:
                text = " ".join(
                    filter(
                        None,
                        [
                            event.get("HOMEDESCRIPTION"),
                            event.get("NEUTRALDESCRIPTION"),
                            event.get("VISITORDESCRIPTION"),
                        ],
                    )
                )
                if "STEAL" in text.upper():
                    offender = real_team(event)
                    others = [team_id for team_id in team_ids if team_id != offender]
                    if len(others) == 1:
                        kind = "STEAL"
                        team = others[0]
            if kind is None or team is None:
                continue
            period = event["PERIOD"]
            start = parse_clock(event["PCTIMESTRING"])
            next_shot = None
            for later in events[index + 1 :]:
                if later["PERIOD"] != period:
                    break
                if int(later["EVENTMSGTYPE"]) in SHOT_EVENTS:
                    next_shot = later
                    break
            if next_shot is None:
                continue
            own_shot[kind][1] += 1
            if real_team(next_shot) != team:
                continue
            elapsed = start - parse_clock(next_shot["PCTIMESTRING"])
            if elapsed < 0.0 or elapsed > 24.0:
                continue
            samples[kind].append((game_id, elapsed))
            own_shot[kind][0] += 1
            if kind == "OREB":
                zone = shot_zones.get((game_id, next_shot["EVENTNUM"]))
                if zone:
                    oreb_zones[zone] += 1
    return samples, own_shot, oreb_zones


def sim_team(player_id):
    if not player_id:
        return None
    return player_id.split("_")[0]


def sim_process(path):
    payload = read_json(path)
    games = payload["events"]
    samples = {"DREB": [], "OREB": [], "STEAL": []}
    own_shot = {"DREB": [0, 0], "OREB": [0, 0], "STEAL": [0, 0]}
    oreb_zones = defaultdict(int)
    for game_index, events in enumerate(games):
        for index, event in enumerate(events):
            kind = None
            team = None
            if event.get("kind") == "REBOUND":
                data = event.get("data", {})
                team = sim_team(data.get("rebounder_id"))
                if data.get("offensive") is True:
                    kind = "OREB"
                elif data.get("offensive") is False:
                    kind = "DREB"
            elif event.get("kind") == "STEAL":
                data = event.get("data", {})
                team = sim_team(data.get("player_id"))
                kind = "STEAL"
            if kind is None or team is None:
                continue
            next_shot = None
            for later in events[index + 1 :]:
                if later.get("kind") == "SHOT":
                    next_shot = later
                    break
            if next_shot is None:
                continue
            own_shot[kind][1] += 1
            shot_team = sim_team(next_shot.get("data", {}).get("shooter_id"))
            if shot_team != team:
                continue
            elapsed = float(next_shot.get("time", 0.0)) - float(event.get("time", 0.0))
            if elapsed < 0.0 or elapsed > 24.0:
                continue
            samples[kind].append((game_index, elapsed))
            own_shot[kind][0] += 1
            if kind == "OREB":
                zone = next_shot.get("data", {}).get("zone")
                if zone in SHOT_ZONES:
                    oreb_zones[zone] += 1
    return samples, own_shot, oreb_zones, len(games)


def mean_ci(clustered, seed, bootstrap):
    if not clustered:
        return None
    values = [value for _, value in clustered]
    groups = defaultdict(list)
    for cluster, value in clustered:
        groups[cluster].append(value)
    keys = list(groups)
    rng = random.Random(seed)
    means = []
    for _ in range(bootstrap):
        picked = [rng.choice(keys) for _ in keys]
        pooled = []
        for key in picked:
            pooled.extend(groups[key])
        means.append(statistics.fmean(pooled))
    means.sort()
    low = means[int(0.025 * len(means))]
    high = means[min(len(means) - 1, int(0.975 * len(means)))]
    return {
        "n": len(values),
        "clusters": len(keys),
        "mean_seconds": statistics.fmean(values),
        "median_seconds": statistics.median(values),
        "ci95_seconds": [low, high],
    }


def share(own, total):
    if total == 0:
        return None
    return own / total


def zone_mix(counts):
    total = sum(counts.values())
    if total == 0:
        return None
    return {zone: counts.get(zone, 0) / total for zone in SHOT_ZONES}


def measured_claim(claim_id, definition, reference, simulation, limitations):
    if not reference or not simulation:
        return {
            "id": claim_id,
            "definition": definition,
            "status": "InsufficientEvidence",
            "league_scope": "NBA",
            "reference": reference,
            "simulation": simulation,
            "reason": "一侧没有可用样本",
            "limitations": limitations,
        }
    delta = simulation["mean_seconds"] - reference["mean_seconds"]
    return {
        "id": claim_id,
        "definition": definition,
        "status": "Measured",
        "league_scope": "NBA",
        "coverage": "event-level play-by-play; same-period elapsed time",
        "reference": reference,
        "simulation": simulation,
        "delta_seconds": delta,
        "limitations": limitations,
    }


def blocked_claim(claim_id, definition, reason, source):
    return {
        "id": claim_id,
        "definition": definition,
        "status": "InsufficientEvidence",
        "league_scope": "NBA",
        "reason": reason,
        "source": source,
    }


def main():
    args = parse_args()
    shot_zones = load_shot_zones(args.shotdetail)
    real_games = real_events(args.pbp)
    real_samples, real_own, real_zones = real_process(real_games, shot_zones)
    sim_samples, sim_own, sim_zones, sim_games = sim_process(args.sim)
    reference_manifest = read_json(args.reference)["reference_manifest"]
    claims = []
    for kind, label in (
        ("DREB", "防守篮板后首次出手"),
        ("OREB", "前场篮板后首次出手"),
        ("STEAL", "抢断后首次出手"),
    ):
        reference = mean_ci(real_samples[kind], args.seed, args.bootstrap)
        simulation = mean_ci(sim_samples[kind], args.seed + 1, args.bootstrap)
        if reference:
            reference["own_shot_share"] = share(real_own[kind][0], real_own[kind][1])
        if simulation:
            simulation["own_shot_share"] = share(sim_own[kind][0], sim_own[kind][1])
        status = "Measured" if reference and simulation else "InsufficientEvidence"
        claims.append(
            measured_claim(
                f"PROC-{kind}-FIRST-SHOT",
                f"{label}到本队下一次出手的时间；先出现对手出手或本段结束记为未形成本队出手。",
                reference,
                simulation,
                [
                    "逐事件归档不含球员观察、传球意图与掩护类型",
                    "时间口径是同一节内比赛钟差，不跨节配对",
                    "模拟样本为显式运行的研究样本，不进入发布门禁",
                ],
            )
        )
    real_mix = zone_mix(real_zones)
    sim_mix = zone_mix(sim_zones)
    claims.append(
        {
            "id": "PROC-OREB-SHOT-ZONE",
            "definition": "前场篮板后本队下一次出手的区域构成（rim/near/mid/three）。",
            "status": "Measured" if real_mix and sim_mix else "InsufficientEvidence",
            "league_scope": "NBA",
            "coverage": "shotdetail 与逐事件归档按 GAME_ID + EVENTNUM 联结",
            "reference": real_mix,
            "simulation": sim_mix,
            "reason": "参考 shotdetail 为 2023-24，逐事件归档为 2024-25，两者比赛身份不相交，不能按事件联结。",
            "limitations": ["只比较构成，不把构成差异当作通过或失败"],
        }
    )
    claims.extend(
        [
            blocked_claim(
                "PROC-PASS-COUNT",
                "每个回合的传球次数与传球结果占比。",
                "逐事件归档不含每次传球与接球人；无法区分传球、手递手与传球被点掉。",
                "nba-shotdetail-pbp-2023-24",
            ),
            blocked_claim(
                "PROC-SCREEN-USE",
                "掩护发生、被利用与后续顺下/外弹的先后。",
                "没有球员追踪或人工标注，掩护类型与使用无法从事件文本可靠辨认。",
                "nba-shotdetail-pbp-2023-24",
            ),
            blocked_claim(
                "PROC-HELP-CLOSE",
                "协防、补位、扑防的触发顺序与窗口时长。",
                "事件归档只有结果事实，没有十人位置与朝向；追踪数据与标注均缺失。",
                "nba-shotdetail-pbp-2023-24",
            ),
            blocked_claim(
                "PROC-CORNER-WINDOW",
                "底角接球窗口的持续时间与被关闭原因。",
                "窗口需要连续位置与防守到达时间；现有归档无法重建。",
                "nba-shotdetail-pbp-2023-24",
            ),
        ]
    )
    report = {
        "study": "process_research_v1",
        "reference": {
            "id": reference_manifest["reference_id"],
            "league": reference_manifest["league"],
            "season": reference_manifest["season"],
            "games": len(real_games),
            "pbp_source": reference_manifest["pbp_source"],
            "pbp_sha256": reference_manifest["pbp_data_sha256"],
            "shotdetail_source": reference_manifest["source"],
            "shotdetail_sha256": reference_manifest["data_sha256"],
            "fit_separation": "归档只用于研究与校准来源，研究脚本不把参考数据送入参数拟合",
        },
        "simulation": {
            "path": args.sim,
            "games": sim_games,
            "note": "显式运行的研究样本；不是发布门禁",
        },
        "claims": claims,
        "coverage": {
            "event_level": ["DREB", "OREB", "STEAL", "OREB shot zone"],
            "missing": ["pass count", "screen use", "help/closeout order", "corner window"],
            "rule": "研究结果不计算总通过率，也不作为发布或自动构建门禁",
        },
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    try:
        out.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"cannot write {out}: {error}") from error
    print(f"wrote {out}")
    for claim in claims:
        status = claim["status"]
        if status == "Measured":
            reference = claim.get("reference") or {}
            simulation = claim.get("simulation") or {}
            if isinstance(reference, dict) and "mean_seconds" in reference:
                print(
                    f"{claim['id']}: {status} reference_n={reference['n']} "
                    f"reference_mean={reference['mean_seconds']:.3f}s "
                    f"simulation_mean={simulation.get('mean_seconds', float('nan')):.3f}s"
                )
            else:
                print(f"{claim['id']}: {status}")
        else:
            print(f"{claim['id']}: {status}")


if __name__ == "__main__":
    main()