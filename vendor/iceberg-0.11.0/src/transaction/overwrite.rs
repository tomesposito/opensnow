// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::spec::{DataFile, ManifestEntry, ManifestFile, Operation};
use crate::table::Table;
use crate::transaction::snapshot::{
    DefaultManifestProcess, SnapshotProduceOperation, SnapshotProducer,
};
use crate::transaction::{ActionCommit, TransactionAction};

/// Atomically replaces live data files with new data files in an overwrite snapshot.
/// Unaffected manifests are reused, and removed files remain available to older snapshots.
pub struct OverwriteAction {
    // below are properties used to create SnapshotProducer when commit
    commit_uuid: Option<Uuid>,
    snapshot_properties: HashMap<String, String>,
    added_data_files: Vec<DataFile>,
    removed_data_files: Vec<DataFile>,
}

impl OverwriteAction {
    pub(crate) fn new() -> Self {
        Self {
            commit_uuid: None,
            snapshot_properties: HashMap::default(),
            added_data_files: vec![],
            removed_data_files: vec![],
        }
    }

    /// Add data files to the snapshot.
    pub fn add_data_files(mut self, data_files: impl IntoIterator<Item = DataFile>) -> Self {
        self.added_data_files.extend(data_files);
        self
    }

    /// Remove live data files from the snapshot. Physical files are retained for time travel.
    /// The commit fails if any file is no longer live or its metadata has changed.
    pub fn delete_data_files(mut self, data_files: impl IntoIterator<Item = DataFile>) -> Self {
        self.removed_data_files.extend(data_files);
        self
    }

    /// Set commit UUID for the snapshot.
    pub fn set_commit_uuid(mut self, commit_uuid: Uuid) -> Self {
        self.commit_uuid = Some(commit_uuid);
        self
    }

    /// Set snapshot summary properties.
    pub fn set_snapshot_properties(mut self, snapshot_properties: HashMap<String, String>) -> Self {
        self.snapshot_properties = snapshot_properties;
        self
    }

    /// Collapse files sharing a path to their first occurrence, so a single
    /// manifest never references the same file twice.
    fn dedupe_added_files(&self) -> Vec<DataFile> {
        let mut seen = HashSet::with_capacity(self.added_data_files.len());
        self.added_data_files
            .iter()
            .filter(|data_file| seen.insert(data_file.file_path.as_str()))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl TransactionAction for OverwriteAction {
    async fn commit(self: Arc<Self>, table: &Table) -> Result<ActionCommit> {
        let snapshot_producer = SnapshotProducer::new(
            table,
            self.commit_uuid.unwrap_or_else(Uuid::now_v7),
            self.snapshot_properties.clone(),
            self.dedupe_added_files(),
        )
        .with_removed_data_files(self.removed_data_files.clone());

        // Validate added files.
        snapshot_producer.validate_added_data_files()?;

        // Check for files already live in the table.
        snapshot_producer.validate_duplicate_files().await?;

        snapshot_producer
            .commit(OverwriteOperation, DefaultManifestProcess)
            .await
    }
}

struct OverwriteOperation;

impl SnapshotProduceOperation for OverwriteOperation {
    fn operation(&self) -> Operation {
        Operation::Overwrite
    }

    async fn delete_entries(
        &self,
        _snapshot_produce: &SnapshotProducer<'_>,
    ) -> Result<Vec<ManifestEntry>> {
        Ok(vec![])
    }

    async fn existing_manifest(
        &self,
        snapshot_produce: &SnapshotProducer<'_>,
    ) -> Result<Vec<ManifestFile>> {
        let Some(snapshot) = snapshot_produce.table.metadata().current_snapshot() else {
            return Ok(vec![]);
        };

        let manifest_list = snapshot_produce
            .table
            .manifest_list_reader(snapshot)
            .load()
            .await?;

        Ok(manifest_list
            .entries()
            .iter()
            .filter(|entry| {
                // Keep delete-only manifests too: they record which files were removed and
                // must persist across snapshots until `expire_snapshots` cleans them up.
                // Dropping them lets the removed files reappear as live data (see #2148).
                entry.has_added_files() || entry.has_existing_files() || entry.has_deleted_files()
            })
            .cloned()
            .collect())
    }
}
