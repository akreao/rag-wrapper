-- run.sh mounts this as an upgrade file: the first statement works, the
-- second fails, like an upstream upgrade that clashes with the database.
CREATE TABLE smoke_partial_upgrade (id int);
ALTER TABLE login ADD COLUMN account_id int;
