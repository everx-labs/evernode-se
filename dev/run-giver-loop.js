const {
    TonClient,
    signerKeys,
    abiContract,
} = require("@eversdk/core");
const { libNode } = require("@eversdk/lib-node");
const fs = require("fs");
const path = require("path");

TonClient.useBinaryLibrary(libNode);

async function main(client) {
    const giver = loadGiver();
    // while (true) {
    process.stdout.write("Sending message to giver... ");
    const result = await client.processing.process_message({
        message_encode_params: {
            address: giver.address,
            abi: giver.abi,
            signer: giver.signer,
            call_set: {
                function_name: "sendTransaction",
                input: {
                    dest: giver.address,
                    value: 1e9,
                    bounce: true,
                },
            },
        },
        send_events: false,
    });
    console.log(`TX: ${result.transaction.id}`);
    await new Promise((resolve) => setTimeout(resolve, 1000));
    // }
}

(async () => {
    try {
        const client = new TonClient({
            network: {
                endpoints: ["http://localhost:3001"],
            },
        });
        await main(client);
        await client.close();
    } catch (err) {
        console.error(err);
        process.exit(1);
    }
})();

function loadGiver() {
    return {
        address: "0:ece57bcc6c530283becbbd8a3b24d3c5987cdddc3c8b7b33be6e4a6312490415",
        abi: abiContract(loadContractsJSON("giver_v2/GiverV2.abi")),
        signer: signerKeys(loadContractsJSON("giver_v2/GiverV2.keys")),
    };
}

function loadContractsJSON(jsonName) {
    return JSON.parse(fs.readFileSync(path.resolve(
        __dirname,
        `../contracts/${jsonName}.json`,
    ), "utf8"));
}

