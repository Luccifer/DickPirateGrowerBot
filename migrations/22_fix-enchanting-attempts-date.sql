-- A row created as a side effect (e.g. by a bless grant) used to get attempts_date = current_date
-- with attempts_left = 0, which blocked the daily attempts grant until the next day.
-- The default is changed to 'epoch' and the already affected rows are healed:
-- attempts_left = 0 can only mean "never granted" at this point, so it is safe.

ALTER TABLE Enchanting ALTER COLUMN attempts_date SET DEFAULT 'epoch';

UPDATE Enchanting SET attempts_date = 'epoch' WHERE attempts_left = 0;
