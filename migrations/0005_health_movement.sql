-- The day's movement around the rowing and the watch's own fitness estimate,
-- fetched from the home server with the recovery data.
ALTER TABLE health_daily ADD COLUMN vo2max REAL;        -- the watch's VO2max estimate, ml/kg/min (Cardio Fitness)
ALTER TABLE health_daily ADD COLUMN strength_min REAL;  -- minutes of strength-training workouts
ALTER TABLE health_daily ADD COLUMN steps REAL;
ALTER TABLE health_daily ADD COLUMN exercise_min REAL;  -- Apple Exercise Time
