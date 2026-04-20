CREATE TABLE IF NOT EXISTS user_mails (
    incr_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    mail_id INTEGER NOT NULL,
    params TEXT NOT NULL DEFAULT '',
    attachment TEXT NOT NULL DEFAULT '',
    state INTEGER NOT NULL DEFAULT 0,
    create_time INTEGER NOT NULL,
    sender TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL DEFAULT '',
    copy TEXT NOT NULL DEFAULT '',
    expire_time INTEGER NOT NULL,
    sender_type INTEGER NOT NULL DEFAULT 0,
    jump_title TEXT NOT NULL DEFAULT '',
    jump TEXT NOT NULL DEFAULT '',
    is_lock BOOLEAN NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_user_mails_user ON user_mails(user_id);
CREATE INDEX IF NOT EXISTS idx_user_mails_expire ON user_mails(expire_time);

CREATE TABLE IF NOT EXISTS user_mail_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    mail_incr_id INTEGER NOT NULL,
    mail_id INTEGER NOT NULL,
    attachment TEXT NOT NULL DEFAULT '',
    action TEXT NOT NULL, -- 'created', 'read', 'claimed', 'expired', 'deleted'
    action_time INTEGER NOT NULL,
    state_at_action INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_user_mail_history_user ON user_mail_history(user_id);
CREATE INDEX IF NOT EXISTS idx_user_mail_history_mail ON user_mail_history(mail_incr_id);
CREATE INDEX IF NOT EXISTS idx_user_mail_history_time ON user_mail_history(action_time);
