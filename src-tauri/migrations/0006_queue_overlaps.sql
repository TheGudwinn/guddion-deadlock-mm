-- How many already installed addons the finished install overlaps with (shown as a warning).
ALTER TABLE download_queue ADD COLUMN overlap_count INT;
