#!/bin/sh -e

TON_NODE="tonlabs/ton-node"
TONOS_SE="${TONOS_SE:-tonlabs/evernode-se}"

BIN_TARGET="evernode_se"

Q_SERVER_GITHUB_REPO="https://github.com/tonlabs/ton-q-server"
Q_SERVER_GITHUB_REV="${Q_SERVER_GITHUB_REV:-master}"

echo
echo "*** Building Evernode SE ***"
echo
docker build \
    --platform linux/amd64 \
    --no-cache \
    --build-arg BIN_TARGET="$BIN_TARGET" \
    --build-arg FEATURES="${1:-disable-tests}" \
    --tag $TON_NODE \
    ./node

echo
echo "*** Building Evernode SE image ***"
echo
docker build \
    --platform linux/amd64 \
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
echo "docker run -d --name evernode-se -e USER_AGREEMENT=yes -p80:80 $TONOS_SE"
