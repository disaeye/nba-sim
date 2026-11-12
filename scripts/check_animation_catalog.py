from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[1]
ACTION_CATALOG_PATH = ROOT / "data" / "rules" / "action_catalog.json"
ANIMATION_CATALOG_PATH = ROOT / "data" / "rules" / "action_animation_catalog.json"

assert ACTION_CATALOG_PATH.is_file(), f"缺少文件: {ACTION_CATALOG_PATH}"
assert ANIMATION_CATALOG_PATH.is_file(), f"缺少文件: {ANIMATION_CATALOG_PATH}"


# pi-lens-ignore: unchecked-throwing-call-python
def load_json(path):
    try:
        raw = path.read_text(encoding="utf-8")
        return json.loads(raw)  # pi-lens-ignore: ast-grep:unchecked-throwing-call-python
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"读取 JSON 失败 {path}: {error}") from error


action_cat = load_json(ACTION_CATALOG_PATH)
anim_cat = load_json(ANIMATION_CATALOG_PATH)

assert anim_cat.get("catalog_version") == 1, "动作动画目录版本不正确"
allowed_clips = set(anim_cat.get("allowed_clips", []))
assert len(allowed_clips) >= 11, "缺少基础动画片段列表"

expected_families = {fam["id"]: fam for fam in action_cat["families"]}
actual_families = {fam["id"]: fam for fam in anim_cat["families"]}

assert set(expected_families.keys()) == set(actual_families.keys()), (
    f"动作族不一致，差异: {set(expected_families.keys()) ^ set(actual_families.keys())}"
)

# 逐族逐动作严格核对
for fam_id, exp_fam in expected_families.items():
    act_fam = actual_families[fam_id]
    assert exp_fam["name"] == act_fam["name"], f"族名称不匹配: {fam_id}"

    exp_actions = set(exp_fam["actions"])
    act_actions_map = {item["name"]: item for item in act_fam["actions"]}
    act_actions = set(act_actions_map.keys())

    assert exp_actions == act_actions, (
        f"动作族 {fam_id} 动作列表不一致，缺少: {exp_actions - act_actions}，多余: {act_actions - exp_actions}"
    )

    for act_name, entry in act_actions_map.items():
        assert entry.get("kind"), f"{fam_id} -> {act_name} 缺少 kind"
        assert entry.get("variant"), f"{fam_id} -> {act_name} 缺少 variant"
        assert entry.get("role"), f"{fam_id} -> {act_name} 缺少 role"
        assert entry.get("phases"), f"{fam_id} -> {act_name} 缺少 phases"

        recipe_3d = entry.get("recipe_3d", {})
        clip = recipe_3d.get("clip")
        assert clip in allowed_clips, f"{fam_id} -> {act_name} 动画片段 {clip} 不合法"
        assert "speed" in recipe_3d, f"{fam_id} -> {act_name} 缺少 speed"
        assert "loop" in recipe_3d, f"{fam_id} -> {act_name} 缺少 loop"

        recipe_2d = entry.get("recipe_2d", {})
        assert recipe_2d.get("kinetics"), f"{fam_id} -> {act_name} 缺少 2D kinetics"
        assert recipe_2d.get("label_zh"), f"{fam_id} -> {act_name} 缺少 2D label_zh"

        scenario_test = entry.get("scenario_test", "")
        assert scenario_test, f"{fam_id} -> {act_name} 缺少 scenario_test"

        if "::" in scenario_test:
            file_part, fn_part = scenario_test.split("::", 1)
            test_file = ROOT / file_part
            assert test_file.is_file(), f"测试文件不存在: {test_file}"
            try:
                test_content = test_file.read_text(encoding="utf-8")
            except OSError as error:
                raise SystemExit(f"读取测试文件失败 {test_file}: {error}") from error
            assert f"fn {fn_part}" in test_content, f"测试函数不存在: {fn_part} 在 {test_file}"

# 动作身份是唯一的：同一个 (族, 类别, 变体, 角色) 不能出现两次，否则
# 两类渲染器对同一帧会给出不同的动画。
identities = set()
for fam_id, fam in actual_families.items():
    for entry in fam["actions"]:
        identity = (fam_id, entry["kind"], entry["variant"], entry["role"])
        assert identity not in identities, f"动作身份重复: {identity}"
        identities.add(identity)

# 动作规格表是引擎与目录之间的桥：它的每个条目都必须在动画目录里有
# 配方，两份清单因此不可能各说各话。
spec_source = ROOT / "crates" / "sim" / "src" / "action_spec.rs"
assert spec_source.is_file(), f"缺少动作规格表: {spec_source}"
try:
    spec_text = spec_source.read_text(encoding="utf-8")
except OSError as error:
    raise SystemExit(f"读取动作规格表失败 {spec_source}: {error}") from error

import re

pattern = re.compile(
    r'push\(ActionFamily::(\w+),\s*"([^"]*)",\s*"([^"]*)",\s*"([^"]*)",\s*ActionRole::(\w+),'
)
family_keys = {
    "ActionDribble": "ACTION-DRIBBLE",
    "ActionDrive": "ACTION-DRIVE",
    "ActionFinish": "ACTION-FINISH",
    "ActionShot": "ACTION-SHOT",
    "ActionPost": "ACTION-POST",
    "ActionPass": "ACTION-PASS",
    "ActionFake": "ACTION-FAKE",
    "ActionBallProtection": "ACTION-BALL-PROTECTION",
    "ActionScreen": "ACTION-SCREEN",
    "ActionCut": "ACTION-CUT",
    "ActionRelocate": "ACTION-RELOCATE",
    "ActionDefenseOnBall": "ACTION-DEFENSE-ON-BALL",
    "ActionDefenseOffBall": "ACTION-DEFENSE-OFF-BALL",
    "ActionCloseout": "ACTION-CLOSEOUT",
    "ActionRebound": "ACTION-REBOUND",
    "ActionTransition": "ACTION-TRANSITION",
}
role_keys = {
    "Primary": "primary",
    "Screener": "screener",
    "CutPartner": "cut_partner",
    "OnBallDefender": "on_ball_defender",
    "Defender": "defender",
    "Rebounder": "rebounder",
    "Shooter": "shooter",
}
spec_identities = set()
for match in pattern.finditer(spec_text):
    family = family_keys.get(match.group(1))
    assert family, f"动作规格表里有未知的动作族: {match.group(1)}"
    role = role_keys.get(match.group(5))
    assert role, f"动作规格表里有未知的角色: {match.group(5)}"
    spec_identities.add((family, match.group(3), match.group(4), role))

assert spec_identities, "动作规格表里没有条目"
assert spec_identities == identities, (
    "动作规格表与动画目录不一致，"
    f"规格表独有: {sorted(spec_identities - identities)}，"
    f"目录独有: {sorted(identities - spec_identities)}"
)

print(
    f"✅ 动作动画覆盖目录校验通过，共 {len(actual_families)} 个动作族、"
    f"{len(identities)} 个动作身份，动作规格表与动画目录完全一致。"
)
