//! 测试资源治理支撑库（gap.md §16.4 / problem.md §14.4）。
//!
//! 为什么需要它：测试如果直接用 `std::env::temp_dir()` + 手动
//! `remove_file`，一旦断言失败（panic）清理代码就不会执行，临时文件
//! 永久残留。多轮测试累积后会把磁盘写满。
//!
//! 本库提供两类保证：
//!
//! 1. [`TempArtifact`]：RAII 临时文件句柄，`Drop` 时无条件删除，
//!    包括 panic 展开路径；
//! 2. [`TempWorkspace`]：每个测试进程一个私有临时目录，并且**进程首次
//!    进入时**会清理该目录中属于本项目的历史残留（同一 process id 的
//!    旧文件、以及名字以本项目前缀开头的孤儿文件）。
//!
//! 纪律：测试中**不得**再出现裸 `std::env::temp_dir()` + 手动删除；
//! 一律使用本库，使泄漏在结构上不可能发生。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 本项目所有临时产物的文件名前缀。用于识别并回收孤儿文件。
pub const TEMP_PREFIX: &str = "nba_";

/// 默认临时根目录（临时文件专用，不放在 /tmp 或 /dev/shm）。
///
/// 为什么单设目录：`/dev/shm` 是内存盘（大流量事件流会耗尽内存），
/// `/tmp` 可能与被清理的构建产物共享分区；专用目录让临时数据与
/// 磁盘配额、清理路径、资源守卫三者边界清晰（problem.md §14.4）。
/// 可由 `NBA_TEST_TMP` 环境变量覆盖（CI/runner 用于统一审计）。
pub const DEFAULT_TEMP_ROOT: &str = "/home/ubuntu/basketball";

/// 单次测试写入的默认上限（字节）。超过即视为资源治理缺陷。
pub const DEFAULT_ARTIFACT_LIMIT_BYTES: u64 = 128 * 1024 * 1024;

fn workspace_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        // 允许外部（CI/runner）通过 NBA_TEST_TMP 指定统一位置，
        // 便于在测试结束后一次性清理与审计。
        let base = std::env::var_os("NBA_TEST_TMP")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TEMP_ROOT));
        let dir = base.join(format!("{TEMP_PREFIX}test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}

/// 每个测试进程独占的临时目录。
///
/// 目录本身在进程首次调用时创建；本函数同时会尝试回收**本项目在本机
/// 临时目录中长期残留的孤儿文件**（仅限匹配前缀且不是当前进程的文件），
/// 从而避免历史泄漏持续累积。
pub fn workspace() -> &'static Path {
    let dir = workspace_dir();
    static RECLAIMED: OnceLock<()> = OnceLock::new();
    RECLAIMED.get_or_init(|| reclaim_orphans(dir));
    dir
}

/// 清理本机临时目录中属于本项目、但不属于当前进程的残留。
///
/// 安全性：只回收「其内嵌进程号已不存在」的条目，避免删掉并行运行的
/// 另一个测试进程正在使用的目录或文件。
fn reclaim_orphans(current: &Path) {
    let Some(parent) = current.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let current_name = current.file_name().map(|n| n.to_os_string());
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        // 只回收本项目前缀的条目，且不碰当前进程自己的目录。
        if !name_str.starts_with(TEMP_PREFIX) || Some(name.clone()) == current_name {
            continue;
        }
        if owner_is_alive(name_str) {
            continue;
        }
        // 目录（如上一轮 workspace）整体删除；文件直接删除。
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// 条目名里是否含有仍然存活的进程号。
///
/// 约定：`TempArtifact` 产物名形如 `nba_<label>.ndjson`（无 pid），
/// `workspace()` 目录形如 `nba_test_<pid>`。对前者保守返回 `true`
/// （可能有并发读者），仅回收 workspace 目录形式。
fn owner_is_alive(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("nba_test_") else {
        return true;
    };
    let Ok(pid) = rest.parse::<u32>() else {
        return true;
    };
    if pid == std::process::id() {
        return true;
    }
    // Linux：/proc/<pid> 存在即视为存活；其他平台保守跳过。
    let proc_path = Path::new("/proc").join(pid.to_string());
    if Path::new("/proc").exists() {
        proc_path.exists()
    } else {
        true
    }
}

/// RAII 临时文件：离开作用域（包含 panic 展开）时删除。
#[derive(Debug)]
pub struct TempArtifact {
    path: PathBuf,
    max_bytes: u64,
}

impl TempArtifact {
    /// 在测试工作区内创建一个临时路径（不预先创建文件）。
    pub fn new(label: &str) -> Self {
        Self::with_limit(label, DEFAULT_ARTIFACT_LIMIT_BYTES)
    }

    /// 指定写入上限的临时路径。
    pub fn with_limit(label: &str, max_bytes: u64) -> Self {
        let safe_label: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let path = workspace().join(format!("{TEMP_PREFIX}{safe_label}.ndjson"));
        Self { path, max_bytes }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 传给需要 `&str` 路径的 API。
    pub fn path_str(&self) -> String {
        self.path.to_string_lossy().to_string()
    }

    /// 断言产物没有超过预算，并返回其字节数。
    ///
    /// 把「生成的数据太大」变成测试失败，而不是磁盘事故。
    pub fn assert_within_limit(&self) -> u64 {
        let bytes = self.size_bytes();
        assert!(
            bytes <= self.max_bytes,
            "temp artifact {} is {} bytes, exceeding the {} byte test budget",
            self.path.display(),
            bytes,
            self.max_bytes
        );
        bytes
    }

    pub fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

impl Drop for TempArtifact {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        // CLI 会在同名路径旁生成评判/违规工件，一并清理。
        for suffix in [
            ".violations.ndjson",
            ".judgments.ndjson",
            ".attribution_report.json",
        ] {
            let _ = std::fs::remove_file(format!("{}{}", self.path.display(), suffix));
        }
    }
}
