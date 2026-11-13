-- Profiles reference mods by (source, remote_id) so they survive export/import between machines.
DROP TABLE profile_mods;
CREATE TABLE profile_mods (
  profile_id BIGINT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  position INT NOT NULL,
  source TEXT NOT NULL,
  remote_id TEXT NOT NULL,
  name TEXT NOT NULL,
  version TEXT,
  enabled BOOLEAN NOT NULL,
  PRIMARY KEY (profile_id, source, remote_id)
);
CREATE UNIQUE INDEX profiles_one_active ON profiles (active) WHERE active;
