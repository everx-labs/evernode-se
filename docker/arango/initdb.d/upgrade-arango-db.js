const COLLECTIONS = {
    blocks: {
        indexes: [
            "workchain_id, shard", "seq_no",
            "workchain_id, seq_no",
            "prev_ref.root_hash,_key",
            "prev_alt_ref.root_hash,_key",
        ],
    },
    accounts: {
        indexes: [],
    },
    messages: {
        indexes: [],
    },
    transactions: {
        indexes: [],
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


