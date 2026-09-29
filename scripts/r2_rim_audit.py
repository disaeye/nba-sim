import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any


def event_payload(event: dict[str, Any], key: str) -> dict[str, Any]:
    data = event.get("data")
    if not isinstance(data, dict) or not isinstance(data.get(key), dict):
        raise ValueError(f"{event.get('kind')} lacks typed {key} payload")
    return data[key]


def distance_to_hoop(position: Any, player_id: str) -> float:
    if not isinstance(position, list) or len(position) != 2:
        raise ValueError("shot position must contain exactly two coordinates")
    x, y = position
    if (
        isinstance(x, bool)
        or not isinstance(x, (int, float))
        or isinstance(y, bool)
        or not isinstance(y, (int, float))
    ):
        raise ValueError("shot position coordinates must be numeric")
    hoop_x = 88.75 if player_id.startswith("H") else 5.25
    dx = x - hoop_x
    dy = y - 25.0
    return (dx * dx + dy * dy) ** 0.5


def validate_source(
    event: dict[str, Any],
    shot: dict[str, Any],
    events_by_id: dict[int, dict[str, Any]],
) -> str:
    source = shot.get("creation_source")
    if not isinstance(source, str):
        raise ValueError("ShotRelease requires creation_source")
    source_id = shot.get("source_event_id")
    parent_id = event.get("parent_event_id")
    parent = events_by_id.get(parent_id) if isinstance(parent_id, int) else None
    transition_id = shot.get("transition_event_id")
    transition_context = shot.get("transition_context")
    if not isinstance(transition_context, bool) or transition_context != (transition_id is not None):
        raise ValueError("transition_context must agree with transition_event_id")
    if transition_id is not None:
        transition = events_by_id.get(transition_id) if isinstance(transition_id, int) else None
        if transition is None or transition.get("kind") != "TRANSITION_STARTED":
            raise ValueError("transition_event_id must identify TRANSITION_STARTED")

    shooter = shot.get("shooter_id")
    if not isinstance(shooter, str):
        raise ValueError("ShotRelease requires shooter_id")
    if source == "drive_finish":
        if parent is None or parent.get("kind") != "DRIVE_REACHED":
            raise ValueError("drive_finish parent must be DRIVE_REACHED")
        outcome = event_payload(parent, "DriveOutcome")
        if not isinstance(outcome.get("successful"), bool):
            raise ValueError("DriveOutcome successful must be boolean")
        if not outcome.get("successful") or outcome.get("driver_id") != shooter:
            raise ValueError("drive_finish parent must be a successful drive by the shooter")
        if source_id != parent.get("event_id"):
            raise ValueError("drive_finish source_event_id must match its parent")
    elif source == "drive_pull_up":
        if parent is None or parent.get("kind") not in {
            "DRIVE_INITIATED", "DRIVE_REACHED", "DRIVE_STOPPED"
        }:
            raise ValueError("drive_pull_up parent must be a drive event")
        payload_key = "DriveInitiated" if parent["kind"] == "DRIVE_INITIATED" else "DriveOutcome"
        drive = event_payload(parent, payload_key)
        if drive.get("driver_id") != shooter or source_id != parent.get("event_id"):
            raise ValueError("drive_pull_up parent identity or source ID does not match")
    elif source == "cut_reception":
        if parent is None or parent.get("kind") != "PASS_RECEIVED":
            raise ValueError("cut_reception parent must be PASS_RECEIVED")
        received = event_payload(parent, "PassReceived")
        if not isinstance(received.get("is_cut_reception"), bool):
            raise ValueError("PassReceived is_cut_reception must be boolean")
        if (
            not received.get("is_cut_reception")
            or received.get("receiver_id") != shooter
            or source_id != parent.get("event_id")
        ):
            raise ValueError("cut_reception parent must identify this shooter's cut catch")
    elif source == "offensive_rebound_putback":
        if parent is None or parent.get("kind") != "REBOUND":
            raise ValueError("putback parent must be REBOUND")
        rebound = event_payload(parent, "ReboundContest")
        if not isinstance(rebound.get("is_offensive"), bool):
            raise ValueError("ReboundContest is_offensive must be boolean")
        if (
            not rebound.get("is_offensive")
            or rebound.get("rebounder_id") != shooter
            or source_id != parent.get("event_id")
        ):
            raise ValueError("putback parent must identify this shooter's offensive rebound")
    elif source == "transition_finish":
        if parent is None or parent.get("kind") != "TRANSITION_STARTED":
            raise ValueError("transition_finish parent must be TRANSITION_STARTED")
        if source_id != parent.get("event_id") or transition_id != parent.get("event_id"):
            raise ValueError("transition_finish IDs must match its causal parent")
    elif source == "set_play":
        if parent is not None or source_id is not None:
            raise ValueError("set_play shots must not have a shot-source parent")
    else:
        raise ValueError(f"unknown ShotRelease creation_source: {source}")
    return source


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: python3 scripts/r2_rim_audit.py FACTS.ndjson")
    lines = Path(sys.argv[1]).read_text(encoding="utf-8").splitlines()
    ticks: list[dict[str, Any]] = []
    for line_number, line in enumerate(lines, 1):
        try:
            tick = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"invalid JSON on line {line_number}") from error
        if not isinstance(tick, dict):
            raise ValueError("every stream line must contain a JSON object")
        ticks.append(tick)
    events = [event for tick in ticks for event in tick.get("event_log", [])]
    if any(not isinstance(event, dict) for event in events):
        raise ValueError("event_log entries must be JSON objects")
    events_by_id: dict[int, dict[str, Any]] = {
        event["event_id"]: event for event in events
    }
    if len(events_by_id) != len(events):
        raise ValueError("event IDs must be unique")

    zones: Counter[str] = Counter()
    sources: Counter[str] = Counter()
    rim_sources: Counter[str] = Counter()
    for event in events:
        if event.get("kind") != "SHOT_RELEASE":
            continue
        shot = event_payload(event, "ShotRelease")
        source = validate_source(event, shot, events_by_id)
        shooter = shot["shooter_id"]
        distance = distance_to_hoop(shot.get("pos"), shooter)
        is_three = shot.get("is_three")
        if not isinstance(is_three, bool):
            raise ValueError("ShotRelease requires a boolean is_three")
        if is_three:
            zone = "three"
        elif distance < 5.0:
            zone = "rim"
            rim_sources[source] += 1
        elif distance < 14.0:
            zone = "near"
        else:
            zone = "mid"
        zones[zone] += 1
        sources[source] += 1

    total = sum(zones.values())
    rim = zones["rim"]
    print(json.dumps({
        "fga": total,
        "zones": {name: zones[name] for name in ("rim", "near", "mid", "three")},
        "rim_share_of_fga": rim / total if total else 0.0,
        "shot_sources": dict(sorted(sources.items())),
        "rim_attempt_sources": dict(sorted(rim_sources.items())),
        "unclassified_rim_attempts": 0,
    }, indent=2))


if __name__ == "__main__":
    main()
