const COLLECTIONS = {
    blocks: {
        indexes: [
            "seq_no, gen_utime",
            "gen_utime",
            "workchain_id, shard", "seq_no",
            "workchain_id, shard", "gen_utime",
            "workchain_id, seq_no",
            "workchain_id, key_block", "seq_no",
            "workchain_id, gen_utime",
            "workchain_id, tr_count", "gen_utime",
            "master.min_shard_gen_utime",
            "prev_ref.root_hash,_key",
            "prev_alt_ref.root_hash,_key",
            "tr_count, gen_utime",
            "chain_order",
            "gen_utime, chain_order",
            "key_block, chain_order",
            "workchain_id, chain_order",
            "workchain_id, shard", "chain_order",
        ],
    },
    accounts: {
        indexes: [
            "last_trans_lt",
            "balance",
            "code_hash, _key",
            "code_hash, balance",
            "last_paid",
        ],
    },
    messages: {
        indexes: [
            "block_id",
            "value, created_at",
            "src, value, created_at",
            "dst, value, created_at",
            "src, created_at",
            "dst, created_at",
            "created_lt",
            "msg_type, created_at",
            "created_at",
            "code_hash, created_at",
            "code_hash, last_paid",
            "src, dst, value, created_at",
            "status, src, created_at, bounced, value",
            "dst, msg_type, created_at, created_lt",
            "src, msg_type, created_at, created_lt",
            "src, dst, value, created_at, created_lt",
            "src, value, msg_type, created_at, created_lt",
            "dst, value, msg_type, created_at, created_lt",
            "src, dst, created_at, created_lt",
            "src, body_hash, created_at, created_lt",
            "chain_order",
            "dst_chain_order",
            "src_chain_order",
            "msg_type, dst_chain_order",
            "msg_type, src_chain_order",
            "dst, dst_chain_order",
            "dst, msg_type, dst_chain_order",
            "dst, msg_type, src, dst_chain_order",
            "src, src_chain_order",
            "src, msg_type, src_chain_order",
            "src, msg_type, dst, src_chain_order",
        ],
    },
    transactions: {
        indexes: [
            "block_id",
            "in_msg",
            "out_msgs[*]",
            "account_addr, now",
            "now",
            "lt",
            "account_addr, orig_status, end_status",
            "now, account_addr, lt",
            "workchain_id, now",
            "block_id, tr_type, outmsg_cnt, now, lt",
            "tr_type, now, lt",
            "account_addr, orig_status, end_status, action.spec_action",
            "account_addr, balance_delta, now, lt",
            "account_addr, lt, now",
            "block_id, lt",
            "balance_delta, now",
            "chain_order",
            "account_addr, chain_order",
            "workchain_id, chain_order",
            "account_addr, aborted, chain_order",
        ],
    },
    blocks_signatures: {
        indexes: [],
    },
    chain_ranges_verification: {
        indexes: [],
        data: [{
            _key: "summary",
            reliable_chain_order_upper_boundary: "z"
        }]
    },
};

function checkBlockchainDb() {
    db._useDatabase("_system");
    if (db._databases().find(x => x.toLowerCase() === "blockchain")) {
        console.log("Database blockchain already exist.");
        return;
    }
    console.log("Database blockchain does not exist. Created.");
    db._createDatabase("blockchain", {}, []);
}


function checkCollection(name, props) {
    db._useDatabase("blockchain");
    let collection = db._collection(name);
    if (!collection) {
        console.log(`Collection ${name} does not exist. Created.`);
        collection = db._create(name);
    } else {
        console.log(`Collection ${name} already exist.`);
    }
    props.indexes.forEach((index) => {
        console.log(`Ensure index ${index}`);
        collection.ensureIndex({
            type: "persistent",
            fields: index.split(",").map(x => x.trim()),
        });
    });

    if (props.data) {
        props.data.forEach((doc) => {
            if (!collection.exists(doc)) {
                collection.insert(doc)
            }
        });
    }
}

function checkCollections(collections) {
    Object.entries(collections).forEach(([name, collection]) => checkCollection(name, collection));
}

checkBlockchainDb();

checkCollections(COLLECTIONS);


