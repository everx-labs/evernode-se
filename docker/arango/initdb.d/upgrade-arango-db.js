function checkBlockchainDb() {
    db._useDatabase('_system');
    if (db._databases().find(x => x.toLowerCase() === 'blockchain')) {
        console.log('Database blockchain already exist.');
        return;
    }
    console.log('Database blockchain does not exist. Created.');
    db._createDatabase('blockchain', {}, []);
}


function checkCollection(name, props) {
    db._useDatabase('blockchain');
    let collection = db._collection(name);
    if (!collection) {
        console.log(`Collection ${name} does not exist. Created.`);
        collection = db._create(name);
    } else {
        console.log(`Collection ${name} already exist.`);
    }
    props.indexes.forEach((index) => {
        console.log(`Ensure index ${index.fields.join(', ')}`);
        collection.ensureIndex(index);
    });

}

function checkCollections(collections) {
    Object.entries(collections).forEach(([name, collection]) => checkCollection(name, collection));
}

function sortedIndex(fields) {
    return {
        type: "persistent",
        fields
    };
}

checkBlockchainDb();

checkCollections({
    blocks: {
        indexes: [
            sortedIndex(['seq_no', 'gen_utime']),
            sortedIndex(['gen_utime']),
            sortedIndex(['workchain_id', 'shard', 'seq_no']),
            sortedIndex(['workchain_id', 'seq_no']),
        ],
    },
    accounts: {
        indexes: [
            sortedIndex(['last_trans_lt']),
            sortedIndex(['balance']),
        ],
    },
    messages: {
        indexes: [
            sortedIndex(['block_id']),
            sortedIndex(['value']),
            sortedIndex(['src', 'created_at']),
            sortedIndex(['dst', 'created_at']),
            sortedIndex(['created_lt']),
            sortedIndex(['created_at']),
        ],
    },
    transactions: {
        indexes: [
            sortedIndex(['block_id']),
            sortedIndex(['in_msg']),
            sortedIndex(['out_msgs[*]']),
            sortedIndex(['account_addr']),
            sortedIndex(['now']),
            sortedIndex(['lt']),
        ],
    },
    blocks_signatures: {
        indexes: [],
    },
});


