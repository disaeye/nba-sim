import csv
import json
import math
import os
import re
import statistics
from collections import Counter, defaultdict

RAW = ".work/calibration/raw"
OUT = "data/calibration"
SHOTS = os.path.join(RAW, "shotdetail_2024.csv")
POSS = os.path.join(RAW, "pbpstats_2024.csv")
SEASON = "2024-25"

# 封盖与抢断的事件文本写法：`X BLOCK (2 BLK): ...` 或 `...: X BLOCK (2 BLK)`。
BLOCK_EVENT_RE = re.compile(r"(\S+) BLOCK \((\d+) BLK\)")
STEAL_EVENT_RE = re.compile(r"(\S+) STEAL \((\d+) STL\)")

KIND_MAP = [
    (r"^Alley Oop", "alley_oop"),
    (r"^(Driving )?Reverse Layup", "reverse_layup"),
    (r"^Driving Finger Roll", "driving_layup"),
    (r"^Driving Layup", "driving_layup"),
    (r"^Cutting Layup", "cutting_layup"),
    (r"^Running Finger Roll", "running_layup"),
    (r"^Running Layup", "running_layup"),
    (r"^Driving Floating", "floater"),
    (r"^Floating", "floater"),
    (r"^Pullup", "pull_up"),
    (r"Running Pull-Up", "pull_up"),
    (r"^Step Back", "step_back"),
    (r"^Turnaround Fadeaway", "turnaround"),
    (r"^Fadeaway", "turnaround"),
    (r"^Turnaround Jump", "turnaround"),
    (r"Hook", "hook"),
    (r"^Driving Dunk", "driving_dunk"),
    (r"^Cutting Dunk", "cutting_dunk"),
    (r"^Running Dunk", "running_dunk"),
    (r"Dunk", "dunk"),
    (r"^Tip", "putback"),
    (r"^Putback", "putback"),
    (r"^Running Jump", "running_jump"),
    (r"^Jump Shot", "catch_shoot"),
    (r"^Jump", "catch_shoot"),
    (r"Layup", "layup"),
]

CREATION_MAP = [
    (r"^Driving", "drive"),
    (r"^Cutting", "cut"),
    (r"^Tip", "putback"),
    (r"^Putback", "putback"),
    (r"^Running", "transition"),
    (r"^Alley Oop", "transition"),
    (r"^Turnaround", "post"),
    (r"^Fadeaway", "post"),
    (r"Hook", "post"),
    (r"^Pullup", "pull_up"),
    (r"^Step Back", "pull_up"),
    (r"^Floating", "pull_up"),
]


def shot_kind(action):
    for pattern, kind in KIND_MAP:
        if re.search(pattern, action):
            return kind
    return "catch_shoot"


def creation_source(action):
    for pattern, source in CREATION_MAP:
        if re.search(pattern, action):
            return source
    return "set"


def zone_of(distance, is_three):
    if is_three:
        return "three"
    if distance < 5.0:
        return "rim"
    if distance < 14.0:
        return "near"
    return "mid"


def sigmoid(x):
    if x > 40.0:
        return 1.0
    if x < -40.0:
        return 0.0
    return 1.0 / (1.0 + math.exp(-x))


def logit(p):
    p = min(max(p, 1e-6), 1.0 - 1e-6)
    return math.log(p / (1.0 - p))


def fit_logistic(rows, iterations=40):
    k = len(rows[0][0])
    beta = [0.0] * (k + 1)
    for _ in range(iterations):
        grad = [0.0] * (k + 1)
        hess = [[0.0] * (k + 1) for _ in range(k + 1)]
        for x, y, w in rows:
            z = beta[0] + sum(beta[i + 1] * x[i] for i in range(k))
            p = sigmoid(z)
            r = (y - p) * w
            grad[0] += r
            for i in range(k):
                grad[i + 1] += r * x[i]
            wq = p * (1.0 - p) * w
            hess[0][0] += wq
            for i in range(k):
                hess[0][i + 1] += wq * x[i]
                hess[i + 1][0] += wq * x[i]
                for j in range(k):
                    hess[i + 1][j + 1] += wq * x[i] * x[j]
        step = solve(hess, grad)
        delta = max(abs(v) for v in step) if step else 0.0
        for i in range(k + 1):
            beta[i] += step[i]
        if delta < 1e-9:
            break
    return beta


def solve(a, b):
    n = len(b)
    m = [row[:] + [b[i]] for i, row in enumerate(a)]
    for col in range(n):
        pivot = max(range(col, n), key=lambda r: abs(m[r][col]))
        if abs(m[pivot][col]) < 1e-12:
            return [0.0] * n
        m[col], m[pivot] = m[pivot], m[col]
        inv = 1.0 / m[col][col]
        for j in range(col, n + 1):
            m[col][j] *= inv
        for r in range(n):
            if r == col:
                continue
            f = m[r][col]
            if f == 0.0:
                continue
            for j in range(col, n + 1):
                m[r][j] -= f * m[col][j]
    return [m[i][n] for i in range(n)]


def load_shots():
    shots = []
    with open(SHOTS, newline="") as handle:
        for row in csv.DictReader(handle):
            distance = float(row["SHOT_DISTANCE"])
            is_three = row["SHOT_TYPE"] == "3PT Field Goal"
            shots.append(
                {
                    "player": row["PLAYER_ID"],
                    "distance": distance,
                    "is_three": is_three,
                    "zone": zone_of(distance, is_three),
                    "kind": shot_kind(row["ACTION_TYPE"]),
                    "creation": creation_source(row["ACTION_TYPE"]),
                    "made": int(row["SHOT_MADE_FLAG"]),
                    "action": row["ACTION_TYPE"],
                }
            )
    return shots


def player_zone_skill(shots):
    """按球员按分区做收缩估计，得到分区命中率与个人离散度。"""
    totals = defaultdict(lambda: defaultdict(lambda: [0, 0]))
    league = defaultdict(lambda: [0, 0])
    for shot in shots:
        zone = shot["zone"]
        totals[shot["player"]][zone][0] += shot["made"]
        totals[shot["player"]][zone][1] += 1
        league[zone][0] += shot["made"]
        league[zone][1] += 1
    league_p = {z: league[z][0] / league[z][1] for z in league}
    skills = {}
    for player, zones in totals.items():
        skills[player] = {}
        for zone, (made, attempts) in zones.items():
            prior = 120.0
            shrunk = (made + prior * league_p[zone]) / (attempts + prior)
            skills[player][zone] = shrunk
    return league_p, skills, totals


DISTANCE_EDGES = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
    20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 32, 35, 40, 60,
]


def distance_bin(distance):
    for upper in DISTANCE_EDGES[1:]:
        if distance < upper:
            return upper
    return DISTANCE_EDGES[-1]


def build_shot_make(shots, league_p, skills, totals):
    kinds = sorted({s["kind"] for s in shots})
    zones = ["rim", "near", "mid", "three"]

    cells = defaultdict(lambda: [0, 0])
    kind_totals = defaultdict(lambda: [0, 0])
    for shot in shots:
        key = (shot["kind"], distance_bin(shot["distance"]), shot["is_three"])
        cells[key][0] += shot["made"]
        cells[key][1] += 1
        kind_totals[(shot["kind"], shot["is_three"])][0] += shot["made"]
        kind_totals[(shot["kind"], shot["is_three"])][1] += 1

    table_cells = defaultdict(lambda: defaultdict(list))
    for (kind, upper, is_three), (made, attempts) in cells.items():
        if attempts < 20:
            continue
        prior = 40.0
        kind_made, kind_attempts = kind_totals[(kind, is_three)]
        kind_p = kind_made / kind_attempts
        rate = (made + prior * kind_p) / (attempts + prior)
        table_cells[kind]["three" if is_three else "two"].append(
            {"upper_ft": upper, "rate": round(rate, 6), "samples": attempts}
        )
    for kind in table_cells:
        for side in table_cells[kind]:
            table_cells[kind][side].sort(key=lambda row: row["upper_ft"])

    # 个人技巧斜率：用未收缩的球员命中率做方差分解。
    # 观测方差 = 真实方差 + 二项抽样方差；属性按 sd=0.15 归一。
    skill_spread = {}
    attribute_sd = 0.15
    for zone in zones:
        observed = []
        for player in totals:
            if zone not in totals[player]:
                continue
            made, attempts = totals[player][zone]
            if attempts < 100:
                continue
            observed.append((made / attempts, attempts))
        if len(observed) < 30:
            skill_spread[zone] = 0.15
            continue
        weight = sum(n for _, n in observed)
        mean_p = sum(p * n for p, n in observed) / weight
        observed_var = sum(n * (p - mean_p) ** 2 for p, n in observed) / weight
        noise_var = sum(p * (1.0 - p) / n for p, n in observed) / len(observed)
        true_var = max(observed_var - noise_var, 1e-8)
        true_sd = math.sqrt(true_var)
        skill_spread[zone] = round(true_sd / attribute_sd, 6)

    table = {
        "table": "shot_make",
        "provenance": {
            "source": "shufinskiy/nba_data shotdetail (stats.nba.com ShotChartDetail)",
            "season": SEASON,
            "sample": len(shots),
            "script": "scripts/calibrate/fit_tables.py",
            "method": "empirical make rate per (shot kind, distance bin, two/three), sparse cells smoothed toward the kind mean",
            "contest": "modeled: centred at 0.5 so the fitted mean is preserved; sensitivity documented in table",
        },
        "distance_edges": DISTANCE_EDGES,
        "cells": {
            kind: {side: table_cells[kind][side] for side in table_cells[kind]}
            for kind in table_cells
        },
        "league_zone_make": {z: round(league_p[z], 6) for z in zones},
        "skill_spread": skill_spread,
        "contest_sensitivity": 1.15,
        "contest_center": 0.30,
        "balance_sensitivity": 0.45,
        "stamina_sensitivity": 0.30,
    }
    return table, kinds, zones


def build_shot_profile(shots, kinds, zones):
    total = len(shots)
    creations = sorted({s["creation"] for s in shots})
    kind_counts = Counter(s["kind"] for s in shots)
    zone_counts = Counter(s["zone"] for s in shots)
    creation_counts = Counter(s["creation"] for s in shots)
    zone_by_kind = defaultdict(Counter)
    distance_by_kind = defaultdict(list)
    three_by_kind = defaultdict(Counter)
    kind_by_creation = defaultdict(Counter)
    zone_by_creation = defaultdict(Counter)
    for shot in shots:
        zone_by_kind[shot["kind"]][shot["zone"]] += 1
        distance_by_kind[shot["kind"]].append(shot["distance"])
        three_by_kind[shot["kind"]][shot["is_three"]] += 1
        kind_by_creation[shot["creation"]][shot["kind"]] += 1
        zone_by_creation[shot["creation"]][shot["zone"]] += 1

    profile = {
        "table": "shot_profile",
        "provenance": {
            "source": "shufinskiy/nba_data shotdetail",
            "season": SEASON,
            "sample": total,
            "script": "scripts/calibrate/fit_tables.py",
            "method": "投篮档案逐球统计出手类型、区域与来源份额，按样本数归一",
        },
        "kind_share": {k: round(kind_counts[k] / total, 6) for k in kinds},
        "zone_share": {z: round(zone_counts[z] / total, 6) for z in zones},
        "three_share": round(sum(1 for s in shots if s["is_three"]) / total, 6),
        "creation_share": {
            c: round(creation_counts[c] / total, 6) for c in creations
        },
        "zone_by_kind": {},
        "distance_by_kind": {},
        "three_share_by_kind": {},
        "kind_by_creation": {},
        "zone_by_creation": {},
    }
    for kind in kinds:
        n = sum(zone_by_kind[kind].values())
        profile["zone_by_kind"][kind] = {
            z: round(zone_by_kind[kind][z] / n, 6) for z in zones
        }
        values = sorted(distance_by_kind[kind])
        profile["distance_by_kind"][kind] = {
            "p10": round(values[int(len(values) * 0.10)], 2),
            "p50": round(values[int(len(values) * 0.50)], 2),
            "p90": round(values[int(len(values) * 0.90)], 2),
            "mean": round(statistics.mean(values), 2),
        }
        profile["three_share_by_kind"][kind] = round(
            three_by_kind[kind][True] / n, 6
        )
    for creation in creations:
        n = sum(kind_by_creation[creation].values())
        profile["kind_by_creation"][creation] = {
            k: round(kind_by_creation[creation][k] / n, 6) for k in kinds
        }
        profile["zone_by_creation"][creation] = {
            z: round(zone_by_creation[creation][z] / n, 6) for z in zones
        }
    return profile


EVENT_SHOT = re.compile(r"(MISS )?([A-Za-z\.'\- ]+?) (\d+)' (.+?)(?: \(|\Z)")
EVENT_REBOUND = re.compile(r"REBOUND \((Off|Def):")


def load_possessions():
    seen = set()
    rows = []
    with open(POSS, newline="") as handle:
        for row in csv.DictReader(handle):
            key = (
                row["GAMEID"],
                row["PERIOD"],
                row["STARTTIME"],
                row["ENDTIME"],
                row["STARTTYPE"],
                row["OPPONENT"],
                row["FG2A"],
                row["FG2M"],
                row["FG3A"],
                row["FG3M"],
                row["TURNOVERS"],
                row["OFFENSIVEREBOUNDS"],
                row["SHOOTINGFOULSDRAWN"],
            )
            if key in seen:
                continue
            seen.add(key)
            rows.append(row)
    return rows


LIVE_ENDINGS = {
    "Off Steal",
    "Off Block",
    "Off At Rim Miss",
    "Off At Rim Block",
    "Off Arc 3 Miss",
    "Off Corner 3 Miss",
    "Off Short Mid-Range Miss",
    "Off Long Mid-Range Miss",
}


def possession_context(start_type):
    return "live" if start_type in LIVE_ENDINGS else "dead"


def build_possession(rows):
    contexts = defaultdict(lambda: {"n": 0, "tov": 0, "fga": 0, "fg2a": 0, "fg2m": 0,
                                    "fg3a": 0, "fg3m": 0, "sf": 0, "oreb": 0,
                                    "durations": []})
    for row in rows:
        ctx = possession_context(row["STARTTYPE"])
        bucket = contexts[ctx]
        bucket["n"] += 1
        bucket["tov"] += int(row["TURNOVERS"])
        bucket["fg2a"] += int(row["FG2A"])
        bucket["fg2m"] += int(row["FG2M"])
        bucket["fg3a"] += int(row["FG3A"])
        bucket["fg3m"] += int(row["FG3M"])
        bucket["sf"] += int(row["SHOOTINGFOULSDRAWN"])
        bucket["oreb"] += int(row["OFFENSIVEREBOUNDS"])
        bucket["fga"] += int(row["FG2A"]) + int(row["FG3A"])
        try:
            start = parse_clock(row["STARTTIME"])
            end = parse_clock(row["ENDTIME"])
            if start is not None and end is not None and start >= end:
                bucket["durations"].append(start - end)
        except ValueError:
            pass

    table = {
        "table": "possession",
        "provenance": {
            "source": "shufinskiy/nba_data pbpstats (pbpstats.com possession data)",
            "season": SEASON,
            "sample": len(rows),
            "script": "scripts/calibrate/fit_tables.py",
            "method": "按回合起始语境分组统计结局速率与时长分位",
            "context": "live = 上一次回合以活球方式终结（抢断/封盖/投失）；dead = 停表方式终结",
        },
        "context": {},
    }
    for ctx, bucket in contexts.items():
        n = bucket["n"]
        durations = sorted(bucket["durations"])
        table["context"][ctx] = {
            "share": round(n / len(rows), 6),
            "turnover_per_possession": round(bucket["tov"] / n, 6),
            "fga_per_possession": round(bucket["fga"] / n, 6),
            "fg2a_per_possession": round(bucket["fg2a"] / n, 6),
            "fg3a_per_possession": round(bucket["fg3a"] / n, 6),
            "shooting_foul_per_possession": round(bucket["sf"] / n, 6),
            "oreb_per_possession": round(bucket["oreb"] / n, 6),
            "fg2_pct": round(bucket["fg2m"] / bucket["fg2a"], 6),
            "fg3_pct": round(bucket["fg3m"] / bucket["fg3a"], 6),
            "duration_p10": round(durations[int(len(durations) * 0.10)], 2),
            "duration_p50": round(durations[int(len(durations) * 0.50)], 2),
            "duration_p90": round(durations[int(len(durations) * 0.90)], 2),
        }
    totals = {
        "n": len(rows),
        "fga": sum(int(r["FG2A"]) + int(r["FG3A"]) for r in rows),
        "fg2a": sum(int(r["FG2A"]) for r in rows),
        "fg2m": sum(int(r["FG2M"]) for r in rows),
        "fg3a": sum(int(r["FG3A"]) for r in rows),
        "fg3m": sum(int(r["FG3M"]) for r in rows),
        "tov": sum(int(r["TURNOVERS"]) for r in rows),
        "oreb": sum(int(r["OFFENSIVEREBOUNDS"]) for r in rows),
        "sf": sum(int(r["SHOOTINGFOULSDRAWN"]) for r in rows),
    }
    table["league"] = {
        "possessions_per_game": round(len(rows) / len({r["GAMEID"] for r in rows}), 4),        "fga_per_possession": round(totals["fga"] / totals["n"], 6),
        "three_attempt_share": round(totals["fg3a"] / totals["fga"], 6),
        "turnover_per_possession": round(totals["tov"] / totals["n"], 6),
        "shooting_foul_per_possession": round(totals["sf"] / totals["n"], 6),
        "oreb_per_possession": round(totals["oreb"] / totals["n"], 6),
        "oreb_per_miss": round(
            totals["oreb"] / (totals["fga"] - totals["fg2m"] - totals["fg3m"]), 6
        ),
        "fg2_pct": round(totals["fg2m"] / totals["fg2a"], 6),
        "fg3_pct": round(totals["fg3m"] / totals["fg3a"], 6),
    }
    free_throw_attempts = 0
    free_throw_makes = 0
    # 封盖与抢断在事件文本里有两种写法：
    #   `X BLOCK (2 BLK): MISS ...` 与 `MISS ...: X BLOCK (2 BLK)`。
    # 按（比赛、球员、计数）去重，同一事件的多行重复不会多算。
    block_events = set()
    steal_events = set()
    for row in rows:
        for line in row["EVENTS"].splitlines():
            for m in BLOCK_EVENT_RE.finditer(line):
                block_events.add((row["GAMEID"], m.group(1), m.group(2)))
            for m in STEAL_EVENT_RE.finditer(line):
                steal_events.add((row["GAMEID"], m.group(1), m.group(2)))
            if "Free Throw" not in line:
                continue
            free_throw_attempts += 1
            if "MISS" not in line:
                free_throw_makes += 1
    games = len({r["GAMEID"] for r in rows})
    table["league"]["fta_per_game"] = round(free_throw_attempts / games, 4)
    table["league"]["shooting_fouls_per_game"] = round(totals["sf"] / games, 4)
    table["league"]["blocks_per_game"] = round(len(block_events) / games, 4)
    table["league"]["steals_per_game"] = round(len(steal_events) / games, 4)
    points = 2 * totals["fg2m"] + 3 * totals["fg3m"] + free_throw_makes
    table["league"]["points_per_game"] = round(points / games, 4)
    return table


def parse_clock(text):
    if not text:
        return None
    parts = text.split(":")
    if len(parts) == 2:
        return int(parts[0]) * 60 + float(parts[1])
    return float(text)


def build_rebound(rows, league_oreb_per_miss):
    zone_miss = Counter()
    zone_oreb = Counter()
    for row in rows:
        events = row["EVENTS"].splitlines()
        first_miss_zone = None
        misses = []
        for line in events:
            match = EVENT_SHOT.search(line)
            if not match:
                continue
            missed = match.group(1) is not None
            distance = float(match.group(3))
            is_three = "3PT" in match.group(4)
            zone = zone_of(distance, is_three)
            if missed:
                misses.append(zone)
                zone_miss[zone] += 1
                if first_miss_zone is None:
                    first_miss_zone = zone
        oreb = int(row["OFFENSIVEREBOUNDS"])
        if oreb > 0 and first_miss_zone is not None:
            zone_oreb[first_miss_zone] += min(oreb, 1)

    total_miss = sum(zone_miss.values())
    total_oreb = sum(zone_oreb.values())
    zone_shares = {zone: zone_miss[zone] / total_miss for zone in zone_miss}
    raw = {
        zone: zone_oreb[zone] / zone_miss[zone]
        for zone in ["rim", "near", "mid", "three"]
        if zone_miss[zone] >= 100
    }
    weighted = sum(raw[zone] * zone_shares[zone] for zone in raw)
    scale = league_oreb_per_miss / weighted if weighted > 0 else 1.0
    table = {
        "table": "rebound",
        "provenance": {
            "source": "shufinskiy/nba_data pbpstats possession table + event text",
            "season": SEASON,
            "sample": total_miss,
            "script": "scripts/calibrate/fit_tables.py",
            "method": "分区相对差异来自事件文本的首次投失归组；总水平取 possession 表的 oreb_per_miss 后按分区投失占比等比缩放",
        },
        "oreb_rate_by_zone": {
            zone: round(raw[zone] * scale, 6) for zone in raw
        },
        "league_oreb_per_miss": round(league_oreb_per_miss, 6),
    }
    return table


def build_freethrow(rows):
    # 罚球命中率不在投篮档案里；用回合事件文本里的 Free Throw 计数估计。
    made = 0
    attempts = 0
    for row in rows:
        for line in row["EVENTS"].splitlines():
            if "Free Throw" not in line:
                continue
            attempts += 1
            if "MISS" not in line:
                made += 1
    table = {
        "table": "freethrow",
        "provenance": {
            "source": "shufinskiy/nba_data pbpstats possession event text",
            "season": SEASON,
            "sample": attempts,
            "script": "scripts/calibrate/fit_tables.py",
            "method": "回合事件文本里的 Free Throw 行逐次计数",
        },
        "league_make": round(made / attempts, 6) if attempts else 0.78,
        "attempts_observed": attempts,
        "skill_slope": 0.26,
        "and_one_share": 0.045,
    }
    return table


def main():
    os.makedirs(OUT, exist_ok=True)
    shots = load_shots()
    league_p, skills, totals = player_zone_skill(shots)
    shot_make, kinds, zones = build_shot_make(shots, league_p, skills, totals)
    profile = build_shot_profile(shots, kinds, zones)
    rows = load_possessions()
    possession = build_possession(rows)
    rebound = build_rebound(rows, possession["league"]["oreb_per_miss"])
    freethrow = build_freethrow(rows)

    for name, table in [
        ("shot_make", shot_make),
        ("shot_profile", profile),
        ("possession", possession),
        ("rebound", rebound),
        ("freethrow", freethrow),
    ]:
        path = os.path.join(OUT, name + ".json")
        with open(path, "w") as handle:
            json.dump(table, handle, indent=2, ensure_ascii=False)
            handle.write("\n")
        print("wrote", path)

    print()
    print("league FG2 %.4f  FG3 %.4f" % (possession["league"]["fg2_pct"], possession["league"]["fg3_pct"]))
    print("FGA/poss %.4f  3PA share %.4f  TOV/poss %.4f  SF/poss %.4f  OREB/miss %.4f" % (
        possession["league"]["fga_per_possession"],
        possession["league"]["three_attempt_share"],
        possession["league"]["turnover_per_possession"],
        possession["league"]["shooting_foul_per_possession"],
        possession["league"]["oreb_per_miss"],
    ))
    print("possessions/game %.2f" % possession["league"]["possessions_per_game"])
    print("FT%% %.4f (%d attempts)" % (freethrow["league_make"], freethrow["attempts_observed"]))
    print("skill spread", shot_make["skill_spread"])
    print("kind share", profile["kind_share"])


main()
