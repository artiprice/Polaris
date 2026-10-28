/// Reference names from NVIDIA's published catalog, not a live availability list.
pub struct ReferenceModel {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub source: &'static str,
}
pub const NVIDIA_MODELS: &[ReferenceModel] = &[
    ReferenceModel {
        id: "nvidia/llama-3.3-nemotron-super-49b-v1.5",
        label: "Nemotron Super 49B v1.5",
        description: "Reasoning and tool calling",
        source: "https://build.nvidia.com/nvidia/llama-3_3-nemotron-super-49b-v1_5",
    },
    ReferenceModel {
        id: "nvidia/nvidia-nemotron-nano-9b-v2",
        label: "Nemotron Nano 9B v2",
        description: "Compact reasoning and agentic tasks",
        source: "https://build.nvidia.com/nvidia/nvidia-nemotron-nano-9b-v2",
    },
    ReferenceModel {
        id: "nvidia/llama-3.1-nemotron-nano-8b-v1",
        label: "Nemotron Nano 8B v1",
        description: "Reasoning for PC and edge",
        source: "https://build.nvidia.com/models?label=Instruction+Following&nim=hosted",
    },
];
pub fn model_label(id: &str) -> &str {
    NVIDIA_MODELS
        .iter()
        .find(|m| m.id == id)
        .map_or(id, |m| m.label)
}
pub fn model_description(id: &str) -> &str {
    NVIDIA_MODELS
        .iter()
        .find(|m| m.id == id)
        .map_or("Configured model", |m| m.description)
}
