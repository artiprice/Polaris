// Unit tests of crates/polaris-platform/crates/process/src/lib.rs (`tests`); compiled into that module via #[path].
use super::pressure_from_memory;
use polaris_sdk::registry::MemoryPressure;

#[test]
fn memory_pressure_threshold_edges_are_inclusive() {
    assert_eq!(pressure_from_memory(5, 100), Ok(MemoryPressure::Critical));
    assert_eq!(pressure_from_memory(6, 100), Ok(MemoryPressure::Elevated));
    assert_eq!(pressure_from_memory(15, 100), Ok(MemoryPressure::Elevated));
    assert_eq!(pressure_from_memory(16, 100), Ok(MemoryPressure::Normal));
    assert!(pressure_from_memory(0, 0).is_err());
}
