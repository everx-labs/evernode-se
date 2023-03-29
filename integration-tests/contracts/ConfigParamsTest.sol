pragma ton-solidity >=0.35.0;
pragma AbiHeader expire;

contract ConfigParamsTest {
    function gasToTon(uint128 gas, int8 wid) public pure returns(uint128 price) {
        tvm.accept();
        return gasToValue(gas, wid);
    }

    function tonToGas(uint128 _ton, int8 wid) public pure returns(uint128 price) {
        tvm.accept();
        return valueToGas(_ton, wid);
    }
}