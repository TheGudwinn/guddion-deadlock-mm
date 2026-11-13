-- What was installed (to detect newer files) and for which game build (to warn after a game patch).
ALTER TABLE mods ADD COLUMN file_id TEXT;
ALTER TABLE mods ADD COLUMN file_md5 TEXT;
ALTER TABLE mods ADD COLUMN game_build TEXT;

-- Previous metadata of a mod before an update, so undoing the update restores it too.
CREATE TABLE update_history (
  batch TEXT PRIMARY KEY,
  mod_id BIGINT NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
  old_version TEXT, old_file_id TEXT, old_file_md5 TEXT, old_game_build TEXT);
