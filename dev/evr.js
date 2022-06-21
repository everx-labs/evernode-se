const {
    TonClient,
    signerKeys,
    abiContract,
    SortDirection,
} = require("@eversdk/core");
const { libNode } = require("@eversdk/lib-node");
const fs = require("fs");
const path = require("path");
const fetch = require("node-fetch");

TonClient.useBinaryLibrary(libNode);

const graphQlEndpoint = "http://localhost:3001";
const seEndpoint = "http://localhost:3000/se";

// const graphQlEndpoint = "http://localhost";
// const seEndpoint = "http://localhost/se";

const evr = new TonClient({
    network: {
        endpoints: [graphQlEndpoint],
    },
});

const giver = {
    address: "0:ece57bcc6c530283becbbd8a3b24d3c5987cdddc3c8b7b33be6e4a6312490415",
    abi: abiContract(loadContractsJSON("giver_v2/GiverV2.abi")),
    signer: signerKeys(loadContractsJSON("giver_v2/GiverV2.keys")),
};

function loadContractsJSON(jsonName) {
    return JSON.parse(fs.readFileSync(path.resolve(
        __dirname,
        `../contracts/${jsonName}.json`,
    ), "utf8"));
}

async function queryLastBlock() {
    return (await evr.net.query_collection({
        collection: "blocks",
        result: "gen_utime seq_no",
        limit: 1,
        order: [
            {
                path: "seq_no",
                direction: SortDirection.DESC,
            },
        ],
        filter: {},
    })).result[0];
}

let timeDeltaSeconds = 0;

async function seControl(command) {
    await fetch(`${seEndpoint}/${command}`, {
        method: "POST",
    });
}

async function resetTime() {
    await seControl("reset-time");
    timeDeltaSeconds = 0;
}

async function increaseTime(delta) {
    await seControl(`increase-time?delta=${delta}`);
    timeDeltaSeconds += delta;
}

function header() {
    const time = Date.now() + timeDeltaSeconds * 1000;
    return {
        time,
        expire: Math.round(time / 1000) + 5,
    };
}

async function topup(address) {
    process.stdout.write(`Topup ${address}... `);
    const result = await evr.processing.process_message({
        message_encode_params: {
            address: giver.address,
            abi: giver.abi,
            signer: giver.signer,
            call_set: {
                header: header(),
                function_name: "sendTransaction",
                input: {
                    dest: address,
                    value: 1e9,
                    bounce: true,
                },
            },
        },
        send_events: true,
    }, (evt) => {
        console.log(evt);
    });
    console.log(`TX: ${result.transaction.id}`);
}


module.exports = {
    evr,
    giver,
    queryLastBlock,
    topup,
    resetTime,
    increaseTime,
};
