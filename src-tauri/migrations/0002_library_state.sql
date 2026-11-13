ALTER TABLE mods ADD COLUMN install_batch TEXT;
ALTER TABLE mods ADD COLUMN state TEXT NOT NULL DEFAULT 'installed'; -- 'installed' | 'removed'
CREATE UNIQUE INDEX mods_source_remote_uq ON mods (source, remote_id) WHERE source IS NOT NULL;
