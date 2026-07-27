-- Откат "фарм-ночи": дюп через /sex-укус из минуса + pvp-минуса + безлимитные микрозаймы.
-- Выполняется автоматически один раз при старте бота (sqlx migrate).
--
-- >>> ПЕРЕД КОММИТОМ СВЕРЬ ИМЕНА И ЧИСЛА С УТРЕННИМ /top <<<
-- Паттерны ниже матчатся по Users.name (отображаемое имя в Telegram, ILIKE, без учёта регистра).
-- Если у кого-то имя не совпадёт ни с одним паттерном — его длина просто не изменится.

-- Маркер отката: по нему бот один раз публикует гневное сообщение ревизора.
CREATE TABLE IF NOT EXISTS Farm_Rollbacks (
    id serial PRIMARY KEY,
    performed_at timestamptz NOT NULL DEFAULT current_timestamp,
    announced boolean NOT NULL DEFAULT false
);

-- 1) Аннулировать все непогашенные микрозаймы (все они взяты во время фарма).
UPDATE Loans SET debt = 0 WHERE repaid_at IS NULL AND debt > 0;

-- 2) Вернуть длины как было. Триггер "раз в день" на время правки отключаем,
--    чтобы он не съел ничьи попытки и не кинул исключение.
ALTER TABLE Dicks DISABLE TRIGGER trg_check_and_update_dicks_timestamp;

UPDATE Dicks d
SET length = v.len
FROM (VALUES
    ('%liza%',      60),  -- Лиза: было ~60 перед фармом
    ('%лиза%',      60),
    ('%anastasia%', 21),  -- Настя: 21 (до боя на 50 в 22:11 было ровно 21)
    ('%наст%',      21),
    ('%vit%',       20),  -- Вит: утреннее значение НЕИЗВЕСТНО, впиши своё!
    ('%вит%',       20),
    ('%gleb%',       6),  -- Глеб: не фармил, оставляем как есть
    ('%глеб%',       6)
) AS v(pattern, len), Users u
WHERE u.name ILIKE v.pattern AND d.uid = u.uid;

-- Страховка: никого не оставляем в минусе.
UPDATE Dicks SET length = 0 WHERE length < 0;

ALTER TABLE Dicks ENABLE TRIGGER trg_check_and_update_dicks_timestamp;

-- 3) Отметить факт отката — анонсер увидит и объявит его в чате.
INSERT INTO Farm_Rollbacks DEFAULT VALUES;
