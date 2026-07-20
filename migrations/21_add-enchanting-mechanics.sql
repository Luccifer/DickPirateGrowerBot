-- Vagina enchanting (Lineage 2 style), bless charges, and gnome raids

CREATE TABLE IF NOT EXISTS Enchanting (
    uid bigint NOT NULL REFERENCES Users(uid) ON DELETE CASCADE,
    chat_id bigint NOT NULL REFERENCES Chats(id) ON DELETE CASCADE,
    sharpness integer NOT NULL DEFAULT 0,
    bless_charges integer NOT NULL DEFAULT 0,
    attempts_left integer NOT NULL DEFAULT 0,
    attempts_date date NOT NULL DEFAULT current_date,
    PRIMARY KEY (chat_id, uid)
);

CREATE TABLE IF NOT EXISTS Bless_Of_Day (
    chat_id bigint NOT NULL REFERENCES Chats(id) ON DELETE CASCADE,
    bless_date date NOT NULL DEFAULT current_date,
    winner_uid bigint NOT NULL,
    amount integer NOT NULL,
    PRIMARY KEY (chat_id, bless_date)
);

CREATE TABLE IF NOT EXISTS Gnome_Raids (
    chat_id bigint NOT NULL REFERENCES Chats(id) ON DELETE CASCADE,
    raid_date date NOT NULL,
    victim_uid bigint NOT NULL,
    stolen integer NOT NULL,
    PRIMARY KEY (chat_id, raid_date)
);
