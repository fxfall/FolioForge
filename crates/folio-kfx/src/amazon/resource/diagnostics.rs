use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct KfxResourceDiagnostic {
    pub code: String,
    pub message: String,
    pub location: Option<String>,
    #[serde(default)]
    pub location_symbol_id: Option<u64>,
    #[serde(default)]
    pub location_source: String,
    pub normalized_location: Option<String>,
    pub location_field_id: u64,
    pub input_index: usize,
    pub container_origin: usize,
    pub external_resource_entity_id: u32,
    pub body_reference_count: usize,
    pub visible_placement_count: usize,
}
