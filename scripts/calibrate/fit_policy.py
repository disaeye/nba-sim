import json
import math
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
PROFILE = os.path.join(ROOT, "data", "calibration", "shot_profile.json")
POSSESSION = os.path.join(ROOT, "data", "calibration", "possession.json")
BINARY = os.path.join(ROOT, "target", "debug", "sim2")

SEEDS = "1..9"


def targets():
    profile = json.load(open(PROFILE))
    possession = json.load(open(POSSESSION))
    league = possession["league"]
    return {
        "creation": profile["creation_share"],
        "zone": profile["zone_share"],
        "possessions_per_game": league["possessions_per_game"],
        "fga_per_possession": league["fga_per_possession"],
        "turnover_per_possession": league["turnover_per_possession"],
        "fta_per_game": league["shooting_foul_per_possession"] * league["possessions_per_game"],
        "points_per_game": 228.4,
    }


def run_sim():
    output = subprocess.run(
        [BINARY, "--seeds", SEEDS, "--json"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    for line in output.stdout.splitlines():
        if line.startswith("METRICS "):
            return json.loads(line[len("METRICS ") :])
    raise RuntimeError("sim2 produced no METRICS line\n" + output.stdout[-2000:])


def ratio(target, actual):
    return (target / actual) if actual > 1e-9 else 1.0


def damp(value, factor, power=0.5):
    return value * (factor**power)


def adjust(weights, metrics, goal):
    creation = metrics["creation_share"]
    w = dict(weights)
    w["shoot_catch_bonus"] = damp(
        w["shoot_catch_bonus"], ratio(goal["creation"]["set"], creation["set"])
    )
    w["shoot_off_dribble_penalty"] = damp(
        w["shoot_off_dribble_penalty"], ratio(creation["pull_up"], goal["creation"]["pull_up"])
    )
    w["drive_base"] = damp(
        w["drive_base"], ratio(goal["creation"]["drive"], creation["drive"])
    )
    w["post_base"] = damp(
        w["post_base"], ratio(goal["creation"]["post"], creation["post"])
    )
    w["cut_base"] = damp(
        w["cut_base"], ratio(goal["creation"]["cut"], creation["cut"])
    )
    pace_factor = ratio(metrics["possessions_per_game"], goal["possessions_per_game"])
    w["dribble_setup"] = damp(w["dribble_setup"], pace_factor)
    w["shoot_early_slack"] = damp(w["shoot_early_slack"], pace_factor)
    w["pass_turnover_rate"] = damp(
        w["pass_turnover_rate"],
        ratio(goal["turnover_per_possession"], metrics["turnover_per_possession"]),
    )
    w["drive_turnover_rate"] = damp(
        w["drive_turnover_rate"],
        ratio(goal["turnover_per_possession"], metrics["turnover_per_possession"]),
    )
    w["shooting_foul_rate"] = damp(
        w["shooting_foul_rate"], ratio(goal["fta_per_game"], metrics["fta_per_game"])
    )
    for key, value in w.items():
        w[key] = min(max(value, 0.02), 6.0)
    return w


def loss(metrics, goal):
    terms = []
    for key, value in goal["creation"].items():
        terms.append((metrics["creation_share"][key] - value) ** 2)
    for key, value in goal["zone"].items():
        terms.append((metrics["zone_share"][key] - value) ** 2)
    terms.append(
        (metrics["possessions_per_game"] / goal["possessions_per_game"] - 1.0) ** 2
    )
    terms.append(
        (metrics["fga_per_possession"] / goal["fga_per_possession"] - 1.0) ** 2
    )
    terms.append(
        (
            metrics["turnover_per_possession"] / goal["turnover_per_possession"] - 1.0
        )
        ** 2
    )
    terms.append((metrics["fta_per_game"] / goal["fta_per_game"] - 1.0) ** 2)
    terms.append((metrics["points_per_game"] / goal["points_per_game"] - 1.0) ** 2)
    return math.sqrt(sum(terms) / len(terms))


def main():
    goal = targets()
    policy = json.load(open(POLICY))
    weights = policy["weights"]
    best = None
    for iteration in range(1, 21):
        metrics = run_sim()
        current = loss(metrics, goal)
        print(
            f"iter {iteration:2d}  loss {current:.4f}  "
            f"poss {metrics['possessions_per_game']:.1f}  "
            f"pts {metrics['points_per_game']:.1f}  "
            f"set {metrics['creation_share']['set']:.3f}  "
            f"drive {metrics['creation_share']['drive']:.3f}  "
            f"pull {metrics['creation_share']['pull_up']:.3f}  "
            f"three {metrics['zone_share']['three']:.3f}"
        )
        if best is None or current < best[0]:
            best = (current, dict(weights))
        if current < 0.02:
            break
        weights = adjust(weights, metrics, goal)
        policy["weights"] = weights
        with open(POLICY, "w") as handle:
            json.dump(policy, handle, indent=2, ensure_ascii=False)
            handle.write("\n")

    policy["weights"] = best[1]
    with open(POLICY, "w") as handle:
        json.dump(policy, handle, indent=2, ensure_ascii=False)
        handle.write("\n")
    print(f"best loss {best[0]:.4f}")
    metrics = run_sim()
    print(json.dumps(metrics, indent=2))


main()
