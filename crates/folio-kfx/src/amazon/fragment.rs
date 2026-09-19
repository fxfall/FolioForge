use std::collections::HashMap;

use super::NativeModel;

/// KFX source identity is fragment type plus its decoded fid. Container origin
/// is retained on each native record as provenance, but is not part of the
/// file-set lookup key; duplicate keys across containers remain collisions.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct FragmentKey {
    pub(crate) fragment_type: u32,
    pub(crate) fid: String,
}

/// Identity index built after the complete logical file set has been parsed.
#[derive(Clone, Debug, Default)]
pub(crate) struct FragmentGraph {
    raw_media_by_key: HashMap<FragmentKey, Vec<usize>>,
}

impl FragmentGraph {
    pub(crate) fn from_native_model(model: &NativeModel) -> Self {
        let mut graph = Self::default();
        for (index, resource) in model.resources.iter().enumerate() {
            if resource.fragment_type != 417 {
                continue;
            }
            let Some(fid) = resource.entity_id.and_then(|entity_id| {
                model
                    .containers
                    .get(resource.container_index)
                    .and_then(|container| container.symbols.names.get(&u64::from(entity_id)))
            }) else {
                continue;
            };
            graph
                .raw_media_by_key
                .entry(FragmentKey {
                    fragment_type: resource.fragment_type,
                    fid: fid.clone(),
                })
                .or_default()
                .push(index);
        }
        graph
    }

    pub(crate) fn find_exact(&self, key: &FragmentKey) -> &[usize] {
        self.raw_media_by_key
            .get(key)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}
