-- Persistent download queue. `downloaded` is only written when a download stops; while it runs the
-- progress lives in memory and the .part file on disk is the truth.
CREATE TABLE download_queue (
  id BIGSERIAL PRIMARY KEY,
  source TEXT NOT NULL, remote_id TEXT NOT NULL,
  mod_name TEXT NOT NULL,
  file_json TEXT NOT NULL,            -- the ModFile (id, name, size, url, md5)
  version TEXT,                       -- page version at the time of queuing
  state TEXT NOT NULL,                -- queued | downloading | paused | installing | done | failed | cancelled
  downloaded BIGINT NOT NULL DEFAULT 0, total BIGINT,
  error TEXT, slot INT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE INDEX download_queue_state ON download_queue (state);
