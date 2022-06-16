const { evr, queryLastBlock, giver, topup, increaseTime } = require("./evr");


(async () => {
    try {
        const startBlockTime = (await queryLastBlock()).gen_utime;
        console.log("Start block time", startBlockTime);
        await topup(giver.address);
        await increaseTime(10000);
        try {
            await topup(giver.address);
        } catch {
        }
        const endBlockTime = (await queryLastBlock()).gen_utime;
        console.log("End block time", endBlockTime);
        console.log("Block time delta", endBlockTime - startBlockTime);
        await evr.close();
    } catch (err) {
        console.error(err);
        process.exit(1);
    }
})();

