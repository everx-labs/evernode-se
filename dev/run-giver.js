const {
    topup,
    giver,
    evr,
} = require("./evr");

(async () => {
    try {
        await topup(giver.address);
        await evr.close();
    } catch (err) {
        console.error(err);
        process.exit(1);
    }
})();

