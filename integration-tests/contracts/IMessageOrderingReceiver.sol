pragma solidity >=0.5.0;

interface IMessageOrderingReceiver {
    function reset() external;
    function acceptMessage(uint32 value) external;
}
