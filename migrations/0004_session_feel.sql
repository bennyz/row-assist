-- How a row felt, from the review screen: the rower's own words, so they
-- aren't in the recording. Re-analysing a session keeps them (it updates
-- the row in place).
ALTER TABLE sessions ADD COLUMN effort INTEGER;   -- 1 (very easy) to 10 (all out)
ALTER TABLE sessions ADD COLUMN comment TEXT;
