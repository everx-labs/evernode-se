#!/bin/sh -e

TON_NODE="tonlabs/ton-node"
TONOS_SE="${TONOS_SE:-tonlabs/local-node}"

BIN_TARGET="ton_node_startup"

Q_SERVER_GITHUB_REPO="https://github.com/tonlabs/ton-q-server"
Q_SERVER_GITHUB_REV="${Q_SERVER_GITHUB_REV:-master}"

echo
echo "*** Building TON Node SE ***"
echo
docker build \
    --no-cache \
    --build-arg BIN_TARGET="$BIN_TARGET" \
    --build-arg FEATURES="${1:-disable-tests}" \
    --tag $TON_NODE \
    ./ton-node-se

echo
echo "*** Building TONOS SE image ***"
echo
docker build \
    --no-cache \
    --build-arg TON_NODE="$TON_NODE" \
    --build-arg Q_SERVER_GITHUB_REPO="$Q_SERVER_GITHUB_REPO" \
    --build-arg Q_SERVER_GITHUB_REV="$Q_SERVER_GITHUB_REV" \
    --tag $TONOS_SE \
    ./docker

echo
echo "BUILD SUCCESS"
echo
echo "How to run:"
echo "docker run -d --name local-node -e USER_AGREEMENT=yes -p80:80 $TONOS_SE"
