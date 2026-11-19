//! 跨进程保存和恢复比赛世界。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{MatchManifest, RuntimeParams, World};

pub type Label = &'static str;

/// 检查点格式版本。
///
/// 每次改变世界的持久结构（新增字段、改变字段含义）都必须升版：旧检查点
/// 带着旧结构，恢复它会让世界少一份状态而不是明确失败。
const VERSION: u32 = 4;
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Serialize)]
struct Save<'a> {
    version: u32,
    manifest: MatchManifest,
    world: &'a World,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Load {
    version: u32,
    manifest: MatchManifest,
    world: World,
}

pub fn save(world: &World, path: &Path) -> Result<(), String> {
    let value = Save {
        version: VERSION,
        manifest: MatchManifest::new(world.setup.clone(), world.seed, &world.params.calibration_version),
        world,
    };
    let value = serde_value::to_value(value).map_err(|error| error.to_string())?;
    let bytes = serde_cbor::to_vec(&value).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("world checkpoint exceeds the byte limit".to_string());
    }
    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())
}

pub fn load(path: &Path, params: &RuntimeParams) -> Result<World, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("world checkpoint exceeds the byte limit".to_string());
    }
    let value: serde_value::Value = serde_cbor::from_slice(&bytes).map_err(|error| error.to_string())?;
    let Load { version, manifest, world } = Load::deserialize(value).map_err(|error| error.to_string())?;
    if version != VERSION {
        return Err(format!("unsupported world checkpoint version {version}"));
    }
    manifest.check_against(params)?;
    manifest.setup.validate()?;
    world.clock.state.validate().map_err(|error| error.to_string())?;
    world.identities.validate().map_err(|error| error.to_string())?;
    if world.seed != manifest.seed
        || world.params.calibration_version != manifest.calibration_version
        || serde_json::to_vec(&world.setup).map_err(|error| error.to_string())?
            != serde_json::to_vec(&manifest.setup).map_err(|error| error.to_string())?
        || serde_json::to_vec(&world.rules).map_err(|error| error.to_string())?
            != serde_json::to_vec(&manifest.setup.rules).map_err(|error| error.to_string())?
    {
        return Err("world checkpoint state does not match its manifest".to_string());
    }
    world.params.validate()?;
    Ok(world)
}

/// 检查点里允许出现的标签数量上限。
///
/// 标签来自引擎发布的枚举与固定理由，但它们是字符串，不是枚举本身。
/// 词表因此按需登记：磁盘内容不能借标签往进程里塞无限多个字符串，
/// 超出上限就地失败。
const MAX_LABELS: usize = 4_096;

/// 已登记的标签词表。
///
/// 标签是 `&'static str`，因此登记时把字符串留在进程里。词表有上限，
/// 一次运行最多留下这些数量的短字符串。
static LABELS: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

fn label_table() -> &'static Mutex<HashMap<String, &'static str>> {
    LABELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 把磁盘上的标签字符串登记成进程里的标签。
fn known<E: serde::de::Error>(value: &str) -> Result<&'static str, E> {
    let mut table = label_table()
        .lock()
        .map_err(|_| E::custom("checkpoint label table is poisoned"))?;
    if let Some(existing) = table.get(value) {
        return Ok(*existing);
    }
    if table.len() >= MAX_LABELS {
        return Err(E::custom(format!(
            "checkpoint label table exceeded {MAX_LABELS} entries"
        )));
    }
    let label: &'static str = Box::leak(value.to_string().into_boxed_str());
    table.insert(value.to_string(), label);
    Ok(label)
}

pub(crate) fn label<'de, D: Deserializer<'de>>(decoder: D) -> Result<&'static str, D::Error> {
    known(&String::deserialize(decoder)?)
}

pub(crate) fn optional_label<'de, D: Deserializer<'de>>(decoder: D) -> Result<Option<&'static str>, D::Error> {
    Option::<String>::deserialize(decoder)?.as_deref().map(known).transpose()
}

pub(crate) fn posture_labels<'de, D: Deserializer<'de>>(decoder: D) -> Result<[&'static str; crate::state::POSTURE_SLOT_COUNT], D::Error> {
    let values = <[String; crate::state::POSTURE_SLOT_COUNT]>::deserialize(decoder)?;
    let mut labels = [""; crate::state::POSTURE_SLOT_COUNT];
    for (slot, value) in labels.iter_mut().zip(values) {
        *slot = known(&value)?;
    }
    Ok(labels)
}
