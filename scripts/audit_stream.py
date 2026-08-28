#!/usr/bin/env python3
import sys
import json
import math

COURT_W = 94.0
COURT_H = 50.0
DT = 0.04
MAX_PLAYER_SPEED = 28.0      # ft/s (Max sprint)
MAX_PLAYER_ACCEL = 200.0     # ft/s^2 (Max acceleration)
MIN_PLAYER_SEP = 3.0         # ft (Anti-clumping body hull)
MAX_BALL_3D_SPEED = 85.0     # ft/s (Fastest bullet pass / rebound tip)

def percentile(sorted_vals, p):
    if not sorted_vals:
        return 0.0
    k = (len(sorted_vals) - 1) * p
    f, c = math.floor(k), math.ceil(k)
    if f == c:
        return sorted_vals[int(k)]
    return sorted_vals[f] * (c - k) + sorted_vals[c] * (k - f)

def main(path):
    p_speeds, p_accels = [], []
    ball_speeds = []
    min_sep = math.inf
    prev_players = None
    prev_p_speeds = {}
    prev_ball = None
    frames = 0
    ball_teleports = 0

    with open(path) as fh:
        for line in fh:
            data = json.loads(line)
            f = data.get("frame", data)
            if "players" not in f:
                continue
            frames += 1
            cur_players = {}
            cur_p_speeds = {}
            pts = []
            
            # 1. Player Kinematics
            for p in f["players"]:
                x, y = p["x"] * COURT_W, p["y"] * COURT_H
                pts.append((x, y))
                pid = p.get("id", p.get("jersey", ""))
                cur_players[pid] = (x, y)
                if prev_players and pid in prev_players:
                    px, py = prev_players[pid]
                    vx = (x - px) / DT
                    vy = (y - py) / DT
                    p_speeds.append(math.hypot(vx, vy))
                    if len(prev_p_speeds) > 0 and pid in prev_p_speeds:
                        pvx, pvy = prev_p_speeds[pid]
                        p_accels.append(math.hypot(vx - pvx, vy - pvy) / DT)
                    cur_p_speeds[pid] = (vx, vy)
            
            for a in range(len(pts)):
                for b in range(a + 1, len(pts)):
                    d = math.hypot(pts[a][0] - pts[b][0], pts[a][1] - pts[b][1])
                    min_sep = min(min_sep, d)
            
            prev_players = cur_players
            prev_p_speeds = cur_p_speeds

            # 2. Ball 3D Kinematics & Continuity
            if "ball" in f and f["ball"]:
                b = f["ball"]
                bx = b["x"] * COURT_W
                by = b["y"] * COURT_H
                bz = b.get("z", 3.0)
                cur_ball = (bx, by, bz)
                if prev_ball is not None:
                    dx = bx - prev_ball[0]
                    dy = by - prev_ball[1]
                    dz = bz - prev_ball[2]
                    dist_3d = math.sqrt(dx*dx + dy*dy + dz*dz)
                    b_speed = dist_3d / DT
                    ball_speeds.append(b_speed)
                    if b_speed > MAX_BALL_3D_SPEED:
                        ball_teleports += 1
                prev_ball = cur_ball

    p_speeds.sort()
    p_accels.sort()
    ball_speeds.sort()

    print(f"=== NBA-SIM 25Hz CONTINUOUS KINEMATICS AUDIT ===")
    print(f"Frames audited: {frames}, Players/frame: {len(cur_players)}")
    print(f"Player p50 speed  = {percentile(p_speeds, 0.50):6.2f} ft/s")
    print(f"Player p99 speed  = {percentile(p_speeds, 0.99):6.2f} ft/s  (limit {MAX_PLAYER_SPEED})")
    print(f"Player max speed  = {p_speeds[-1] if p_speeds else 0:6.2f} ft/s")
    print(f"Player p50 accel  = {percentile(p_accels, 0.50):6.2f} ft/s^2")
    print(f"Player p99 accel  = {percentile(p_accels, 0.99):6.2f} ft/s^2  (limit {MAX_PLAYER_ACCEL})")
    print(f"Player max accel  = {p_accels[-1] if p_accels else 0:6.2f} ft/s^2")
    print(f"Player min sep    = {min_sep:.4f} ft  (floor {MIN_PLAYER_SEP})")
    print(f"Ball 3D p50 speed = {percentile(ball_speeds, 0.50):6.2f} ft/s")
    print(f"Ball 3D p99 speed = {percentile(ball_speeds, 0.99):6.2f} ft/s  (limit {MAX_BALL_3D_SPEED})")
    print(f"Ball 3D max speed = {ball_speeds[-1] if ball_speeds else 0:6.2f} ft/s")
    print(f"Ball teleport breaches (> {MAX_BALL_3D_SPEED} ft/s): {ball_teleports}")

    ok = (
        percentile(p_speeds, 0.99) <= MAX_PLAYER_SPEED and
        percentile(p_accels, 0.99) <= MAX_PLAYER_ACCEL and
        min_sep >= MIN_PLAYER_SEP and
        ball_teleports == 0
    )
    print(f"AUDIT RESULT: {'PASS' if ok else 'FAIL'}")
    sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "web/game.ticks.ndjson")
