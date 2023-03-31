pragma ton-solidity >=0.35.0;

contract ConfigParamsTest {
    function returnNow() public pure returns (uint64) {
        tvm.accept();
        return (now);
    }
}
