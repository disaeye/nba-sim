import hashlib
import json
from pathlib import Path

import numpy as np
import pandas as pd


def main():
    archive = Path(".work/r7/shotdetail_2023.tar.xz")
    source_csv = Path(".work/r7/shotdetail_2023.csv")
    fixture_path = Path("crates/evaluator/fixtures/nba.v3.json")
    archive_hash = hashlib.sha256(archive.read_bytes()).hexdigest()
    fixture = json.loads(fixture_path.read_text())
    reference = fixture["joint_situational_bands"]
    provenance = reference["provenance"]
    assert archive_hash == provenance["source_sha256"], "NBA reference archive SHA-256 mismatch"
    benchmark = provenance["external_check"]
    assert benchmark["metric"] == "league 3PA/FGA"
    assert abs(benchmark["value"] - 0.395) < 1e-12
    assert provenance["season"] == "2023-24 NBA regular season"
    assert "f4001f944d4aba1ba63e3d70edb3d6135ba576fe" in provenance["source_version"]
    assert reference["shot_zone_make"]["source_season"] == provenance["season"]

    frame = pd.read_csv(source_csv)
    attempts = frame.loc[frame["SHOT_ATTEMPTED_FLAG"] == 1].copy()
    attempts["shot_distance_ft"] = pd.to_numeric(attempts["SHOT_DISTANCE"], errors="coerce")
    attempts["loc_x_ft"] = pd.to_numeric(attempts["LOC_X"], errors="coerce") / 10.0
    attempts["loc_y_ft"] = pd.to_numeric(attempts["LOC_Y"], errors="coerce") / 10.0
    attempts["period"] = pd.to_numeric(attempts["PERIOD"], errors="coerce")
    attempts["seconds_left"] = (
        pd.to_numeric(attempts["MINUTES_REMAINING"], errors="coerce") * 60
        + pd.to_numeric(attempts["SECONDS_REMAINING"], errors="coerce")
    )
    attempts["made"] = pd.to_numeric(attempts["SHOT_MADE_FLAG"], errors="coerce")
    attempts["is_three"] = attempts["SHOT_TYPE"].eq("3PT Field Goal")

    assert len(attempts) == provenance["attempts"], "shot-attempt count differs from fixture metadata"
    assert attempts["GAME_ID"].nunique() == provenance["games"], "game count differs from fixture metadata"
    assert not attempts.duplicated(["GAME_ID", "GAME_EVENT_ID"]).any(), "duplicate game event ids"
    assert not attempts[["shot_distance_ft", "loc_x_ft", "loc_y_ft", "made", "period", "seconds_left"]].isna().any().any(), (
        "source contains missing analysis fields"
    )
    shot_x = 88.75 - attempts["loc_y_ft"]
    shot_y = 25.0 + attempts["loc_x_ft"]
    radius = np.hypot(shot_x - 88.75, shot_y - 25.0)
    near_sideline = (shot_y <= 3.0) | (shot_y >= 47.0)
    in_attacking_half = shot_x >= 47.0
    geometric_three = (radius >= 23.75) | (
        near_sideline & in_attacking_half & (radius >= 22.0)
    )
    source_three = attempts["is_three"].copy()
    attempts["is_three"] = source_three
    attempts["geometry_three"] = geometric_three
    attempts["zone_is_three"] = source_three | geometric_three
    attempts["radius_ft"] = radius
    source_three_share = float(source_three.mean())
    assert abs(source_three_share - benchmark["value"]) < 0.001, (
        "archive three-point attempt share fails independent NBA benchmark"
    )
    assert np.allclose(np.floor(radius + 1e-9), attempts["shot_distance_ft"]), (
        "shot coordinates do not reproduce NBA ShotChartDetail shot-distance bins"
    )

    zones = reference["shot_zone_make"]
    zone_names = ["rim", "near", "mid", "three"]
    measured_zone_bands = {}
    source_zone_minima = {}
    for zone in zone_names:
        selected = attempts["zone_is_three"] if zone == "three" else ~attempts["zone_is_three"]
        distance = attempts["radius_ft"]
        if zone == "rim":
            selected = selected & (distance < 5)
        elif zone == "near":
            selected = selected & distance.between(5, 14, inclusive="left")
        elif zone == "mid":
            selected = selected & (distance >= 14)
        per_game = attempts.loc[selected].groupby("GAME_ID")["made"].agg(["sum", "count"])
        source_zone_minima[zone] = int(per_game["count"].min())
        assert len(per_game) == provenance["games"], f"some games have no {zone} attempts"
        assert source_zone_minima[zone] >= zones["source_minimum_attempts_per_zone"]
        assert source_zone_minima[zone] >= zones["minimum_attempts_per_zone"]
        rates = per_game["sum"] / per_game["count"]
        measured = {"min": float(rates.quantile(0.05)), "max": float(rates.quantile(0.95))}
        measured_zone_bands[zone] = measured
        assert abs(measured["min"] - zones[zone]["min"]) < 1e-12
        assert abs(measured["max"] - zones[zone]["max"]) < 1e-12

    late = attempts.loc[(attempts["period"] == 4) & (attempts["seconds_left"] <= 300)]
    early = attempts.loc[(attempts["period"] == 4) & (attempts["seconds_left"] > 300)]
    late_stats = late.groupby("GAME_ID").agg(late_fga=("made", "size"), late_3pa=("is_three", "sum"))
    early_stats = early.groupby("GAME_ID").agg(early_fga=("made", "size"), early_3pa=("is_three", "sum"))
    q4 = reference["q4_late_three_attempt_share"]
    source_late_min = int(late_stats["late_fga"].min())
    source_early_min = int(early_stats["early_fga"].min())
    assert source_late_min >= q4["source_minimum_late_attempts"]
    assert source_early_min >= q4["source_minimum_early_attempts"]
    paired = late_stats.join(early_stats, how="inner")
    paired = paired.loc[
        (paired["late_fga"] >= q4["minimum_attempts_per_window"])
        & (paired["early_fga"] >= q4["minimum_attempts_per_window"])
    ].copy()
    assert len(paired) == provenance["games"], "not all source games meet the Q4 sample threshold"
    paired["delta"] = (
        paired["late_3pa"] / paired["late_fga"]
        - paired["early_3pa"] / paired["early_fga"]
    )
    measured_delta = {
        "min": float(paired["delta"].quantile(0.05)),
        "max": float(paired["delta"].quantile(0.95)),
    }
    assert abs(measured_delta["min"] - q4["delta"]["min"]) < 1e-12
    assert abs(measured_delta["max"] - q4["delta"]["max"]) < 1e-12

    coordinate_source_disagreements = int(
        geometric_three.ne(source_three).sum()
    )
    assert coordinate_source_disagreements == 22, (
        "coordinate and ShotChartDetail point-value disagreements changed"
    )

    print(json.dumps({
        "archive_sha256": archive_hash,
        "games": int(attempts["GAME_ID"].nunique()),
        "attempts": len(attempts),
        "source_three_point_attempt_share": source_three_share,
        "geometry_three_point_attempt_share": float(attempts["geometry_three"].mean()),
        "zone_three_point_attempt_share": float(attempts["zone_is_three"].mean()),
        "independent_three_point_attempt_share": benchmark["value"],
        "coordinate_source_three_disagreements": coordinate_source_disagreements,
        "source_zone_minimum_attempts": source_zone_minima,
        "geometry_zone_game_fg_pct_5_95": measured_zone_bands,
        "q4_source_minimum_fga": {"late": source_late_min, "early": source_early_min},
        "q4_games_after_minimum_sample_filter": len(paired),
        "q4_delta_5_95": measured_delta,
    }, indent=2))


if __name__ == "__main__":
    main()
