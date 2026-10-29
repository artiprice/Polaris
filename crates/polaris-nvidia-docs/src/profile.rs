//! `$nvidia(auto)`: which NVIDIA stacks and GPU architectures a workspace and a prompt involve.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use crate::NvidiaError;

/// Most directory entries examined per profile.
const MAX_ENTRIES: usize = 5_000;
/// Deepest directory level examined.
const MAX_DEPTH: usize = 6;
/// Largest build or dependency file read, in bytes.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Directories that never describe the project's own stack.
const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    ".polaris",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "build",
    "dist",
];

/// A documentation set Polaris can consult.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DocTopic {
    /// CUDA C++ programming and runtime.
    Cuda,
    /// PTX ISA.
    Ptx,
    /// TensorRT-LLM.
    TensorRtLlm,
    /// Triton Inference Server.
    Triton,
    /// NCCL collectives.
    Nccl,
    /// NVIDIA OpenShell sandboxes.
    OpenShell,
    /// NemoClaw.
    NemoClaw,
    /// Nemotron models.
    Nemotron,
}

impl DocTopic {
    /// Every topic.
    pub const ALL: [Self; 8] = [
        Self::Cuda,
        Self::Ptx,
        Self::TensorRtLlm,
        Self::Triton,
        Self::Nccl,
        Self::OpenShell,
        Self::NemoClaw,
        Self::Nemotron,
    ];

    /// Stable name used in seeds and directives.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Ptx => "ptx",
            Self::TensorRtLlm => "tensorrt-llm",
            Self::Triton => "triton",
            Self::Nccl => "nccl",
            Self::OpenShell => "openshell",
            Self::NemoClaw => "nemoclaw",
            Self::Nemotron => "nemotron",
        }
    }

    /// Parse [`Self::as_str`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|topic| topic.as_str() == name)
    }

    /// Lowercase words that name the topic in prose.
    const fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Cuda => &["cuda", "__global__", "threadidx", "warp", "cudamalloc"],
            Self::Ptx => &["ptx", "ptxas", "sass"],
            Self::TensorRtLlm => &["tensorrt-llm", "tensorrt_llm", "trt-llm", "tensorrt"],
            Self::Triton => &["triton"],
            Self::Nccl => &["nccl", "allreduce", "all-reduce"],
            Self::OpenShell => &["openshell"],
            Self::NemoClaw => &["nemoclaw"],
            Self::Nemotron => &["nemotron"],
        }
    }
}

/// A CUDA compute capability such as `sm_75`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComputeCapability {
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
}

impl ComputeCapability {
    /// From the two- or three-digit form (`75`, `90`, `100`).
    #[must_use]
    pub fn from_digits(digits: &str) -> Option<Self> {
        let value: u16 = digits.parse().ok()?;
        if !(10..=199).contains(&value) {
            return None;
        }
        Some(Self {
            major: u8::try_from(value / 10).ok()?,
            minor: u8::try_from(value % 10).ok()?,
        })
    }

    /// From the dotted form (`7.5`).
    #[must_use]
    pub fn from_dotted(text: &str) -> Option<Self> {
        let (major, minor) = text.trim().split_once('.')?;
        Some(Self {
            major: major.trim().parse().ok()?,
            minor: minor.trim().trim_end_matches(['+', 'a']).parse().ok()?,
        })
    }
}

impl fmt::Display for ComputeCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sm_{}{}", self.major, self.minor)
    }
}

/// A local GPU as `nvidia-smi` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    /// Product name.
    pub name: String,
    /// Compute capability.
    pub capability: ComputeCapability,
}

/// What a workspace and prompt involve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NvidiaProfile {
    /// Documentation sets worth consulting.
    pub topics: BTreeSet<DocTopic>,
    /// Architectures the build targets.
    pub target_archs: BTreeSet<ComputeCapability>,
    /// Why each topic or architecture was chosen (`path: reason` or `prompt: word`).
    pub evidence: Vec<String>,
}

impl NvidiaProfile {
    /// Notes where local GPUs cannot run what the build targets.
    #[must_use]
    pub fn hardware_notes(&self, local: &[Gpu]) -> Vec<String> {
        let Some(best) = local.iter().map(|gpu| gpu.capability).max() else {
            return Vec::new();
        };
        self.target_archs
            .iter()
            .filter(|target| **target > best)
            .map(|target| {
                format!(
                    "the build targets {target}, but the newest local GPU is {best}: test {target}-only features (for example TMA, wgmma or FP8 on sm_90) on the target machine"
                )
            })
            .collect()
    }
}

/// Profile `workspace` and `prompt`.
/// # Errors
/// The workspace cannot be listed.
pub fn auto_profile(workspace: &Path, prompt: &str) -> Result<NvidiaProfile, NvidiaError> {
    let mut profile = NvidiaProfile::default();
    let lowered = prompt.to_lowercase();
    for topic in DocTopic::ALL {
        if let Some(word) = topic
            .keywords()
            .iter()
            .find(|word| lowered.contains(**word))
        {
            profile.topics.insert(topic);
            profile.evidence.push(format!("prompt: {word}"));
        }
    }
    for capability in arch_mentions(prompt) {
        if profile.target_archs.insert(capability) {
            profile.evidence.push(format!("prompt: {capability}"));
        }
    }
    let mut stack = vec![(workspace.to_owned(), 0_usize)];
    let mut seen = 0_usize;
    while let Some((directory, depth)) = stack.pop() {
        let entries = std::fs::read_dir(&directory).map_err(|_| NvidiaError::Workspace)?;
        for entry in entries {
            seen = seen.saturating_add(1);
            if seen > MAX_ENTRIES {
                return Ok(profile);
            }
            let Ok(entry) = entry else { continue };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH && !SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                    stack.push((path, depth.saturating_add(1)));
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(workspace)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            examine(&mut profile, &path, &relative, &name);
        }
    }
    Ok(profile)
}

fn examine(profile: &mut NvidiaProfile, path: &Path, relative: &str, name: &str) {
    let lowered = name.to_lowercase();
    let extension = lowered
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default();
    let note = |profile: &mut NvidiaProfile, topic: DocTopic, why: &str| {
        if profile.topics.insert(topic) {
            profile.evidence.push(format!("{relative}: {why}"));
        }
    };
    match extension {
        "cu" | "cuh" => note(profile, DocTopic::Cuda, "CUDA source"),
        "ptx" => note(profile, DocTopic::Ptx, "PTX module"),
        _ => {}
    }
    let build_file = matches!(
        lowered.as_str(),
        "cmakelists.txt" | "makefile" | "setup.py" | "build.rs" | "meson.build"
    ) || matches!(extension, "cmake" | "mk" | "sh" | "ps1");
    let dependency_file = lowered.starts_with("requirements") && extension == "txt"
        || matches!(
            lowered.as_str(),
            "pyproject.toml" | "environment.yml" | "environment.yaml" | "cargo.toml"
        );
    if !(build_file || dependency_file || matches!(extension, "cu" | "cuh")) {
        return;
    }
    let Some(text) = read_small(path) else { return };
    let text_lower = text.to_lowercase();
    if build_file && (text_lower.contains("cuda") || text_lower.contains("nvcc")) {
        note(profile, DocTopic::Cuda, "build uses CUDA");
    }
    if dependency_file {
        for (needle, topic) in [
            ("tensorrt_llm", DocTopic::TensorRtLlm),
            ("tensorrt-llm", DocTopic::TensorRtLlm),
            ("tritonclient", DocTopic::Triton),
            ("triton", DocTopic::Triton),
            ("nccl", DocTopic::Nccl),
            ("cupy", DocTopic::Cuda),
            ("cudarc", DocTopic::Cuda),
        ] {
            if text_lower.contains(needle) {
                note(profile, topic, needle);
            }
        }
    }
    if text_lower.contains("nccl") && matches!(extension, "cu" | "cuh") {
        note(profile, DocTopic::Nccl, "NCCL calls");
    }
    for capability in arch_mentions(&text) {
        if profile.target_archs.insert(capability) {
            profile.evidence.push(format!("{relative}: {capability}"));
        }
    }
}

fn read_small(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    String::from_utf8(std::fs::read(path).ok()?).ok()
}

/// Architectures named as `sm_75`, `compute_90`, `sm_90a`, CMake `CUDA_ARCHITECTURES 75;90` or
/// `TORCH_CUDA_ARCH_LIST="7.5;9.0"`.
#[must_use]
pub fn arch_mentions(text: &str) -> BTreeSet<ComputeCapability> {
    let mut found = BTreeSet::new();
    let lowered = text.to_lowercase();
    for prefix in ["sm_", "compute_"] {
        for (at, _) in lowered.match_indices(prefix) {
            let digits: String = lowered
                .get(at.saturating_add(prefix.len())..)
                .unwrap_or_default()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Some(capability) = ComputeCapability::from_digits(&digits) {
                found.insert(capability);
            }
        }
    }
    for line in lowered.lines() {
        if let Some(at) = line.find("cuda_architectures") {
            let values = line
                .get(at.saturating_add("cuda_architectures".len())..)
                .unwrap_or_default();
            for word in values.split(|ch: char| !ch.is_ascii_digit()) {
                if let Some(capability) = ComputeCapability::from_digits(word) {
                    found.insert(capability);
                }
            }
        }
        if let Some(at) = line.find("torch_cuda_arch_list") {
            let values = line
                .get(at.saturating_add("torch_cuda_arch_list".len())..)
                .unwrap_or_default();
            for word in values.split(|ch: char| !(ch.is_ascii_digit() || ch == '.')) {
                if let Some(capability) = ComputeCapability::from_dotted(word) {
                    found.insert(capability);
                }
            }
        }
    }
    found
}

/// Parse `nvidia-smi --query-gpu=name,compute_cap --format=csv,noheader` output.
#[must_use]
pub fn parse_nvidia_smi(output: &str) -> Vec<Gpu> {
    output
        .lines()
        .filter_map(|line| {
            let (name, capability) = line.rsplit_once(',')?;
            Some(Gpu {
                name: name.trim().to_owned(),
                capability: ComputeCapability::from_dotted(capability)?,
            })
        })
        .collect()
}
