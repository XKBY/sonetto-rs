CREATE TABLE user_pool_pop_up_infos (
    user_id INTEGER NOT NULL,
    pool_id INTEGER NOT NULL,
    order_id INTEGER NOT NULL,
    recommend_pop_up_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, pool_id, order_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_user_pool_pop_up_infos ON user_pool_pop_up_infos(user_id);
