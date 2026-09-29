-- RawImportJson was stored for "round-trip export fidelity" but never read - the exporter
-- builds fresh JSON from current field values. Drop the dead column.
ALTER TABLE Characters DROP COLUMN RawImportJson;
