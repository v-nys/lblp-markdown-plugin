use extism_pdk::*;
use logic_based_learning_paths::domain_without_loading::{
    BoolPayload, ClusterProcessingPayload, ClusterProcessingResult,
};
use std::collections::HashSet;

#[host_fn]
extern "ExtismHost" {
    fn file_exists(path: String) -> BoolPayload;
}

#[plugin_fn]
pub fn process_cluster(
    _cpp: ClusterProcessingPayload,
) -> FnResult<ClusterProcessingResult> {
    let artifacts = HashSet::new();
    Ok(ClusterProcessingResult {
        hash_set: artifacts,
    })
}
