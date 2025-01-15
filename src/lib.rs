use extism_pdk::*;
use logic_based_learning_paths::domain_without_loading::{
    BoolPayload, ClusterProcessingPayload, ClusterProcessingResult, DirectoryStructurePayload, DummyPayload, FileWriteOperationPayload, ParamsSchema, SystemTimePayload
};
use std::collections::{HashSet,HashMap};
use serde_json;

#[host_fn]
extern "ExtismHost" {
    // fn file_exists(relative_path: String) -> BoolPayload;
    fn get_system_time() -> SystemTimePayload;
    fn get_last_modification_time(relative_path: String) -> SystemTimePayload;
    // can't use multiple String args here!
    fn write_text_file(payload: FileWriteOperationPayload) -> ();
    // something to get folder structure for the cluster, including nested dirs and filenames
    fn get_cluster_structure(payload: DummyPayload) -> DirectoryStructurePayload;
}

#[plugin_fn]
pub fn get_params_schema(_: ()) -> FnResult<ParamsSchema> {
    let mut parameters = HashMap::new();
    let string_schema = schemars::schema_for!(String);
    let value = serde_json::to_value(string_schema).expect("Should be convertible.");
    parameters.insert("input_extension".into(), (true, value.clone()));
    parameters.insert("output_extension".into(), (true, value));
    let boolean_schema = schemars::schema_for!(bool);
    let value = serde_json::to_value(boolean_schema).expect("Should be convertible.");
    parameters.insert("include_artifact_mapping".into(), (true, value));
    Ok(ParamsSchema { schema: parameters })
}

#[plugin_fn]
pub fn process_cluster(_cpp: ClusterProcessingPayload) -> FnResult<ClusterProcessingResult> {
    let artifacts = HashSet::new();
    let DirectoryStructurePayload { entries } = (unsafe { get_cluster_structure(DummyPayload {}) }).expect("Thought this would be fine.");
    let payload = FileWriteOperationPayload  { 
        relative_path: "md_rendering_test".into(),
        contents: format!("{entries:#?}")
    };
    let write_result = unsafe { write_text_file(payload) }?;
    // should include mapping for converted files iff this plugin is meant as "terminator"
    // i.e. if further processing of HTML is expected, don't include
    Ok(ClusterProcessingResult {
        hash_set: artifacts,
    })
}
