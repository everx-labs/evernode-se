ARG TON_NODE

FROM node:14-buster as q-server-build

ARG Q_SERVER_GITHUB_REPO
ARG Q_SERVER_GITHUB_REV

RUN apt-get update && apt-get install git

# Install Q-Server
USER node
WORKDIR /home/node
RUN git clone --recursive --branch $Q_SERVER_GITHUB_REV $Q_SERVER_GITHUB_REPO ton-q-server
WORKDIR /home/node/ton-q-server
RUN npm ci; \
    npm run tsc; \
    npm ci --production

FROM ${TON_NODE} as ton-node

FROM arangodb:3.7.11

ENV LANG=en_US.UTF-8
ENV PYTHONIOENCODING=utf-8

RUN rm /entrypoint.sh

COPY --from=q-server-build /home/ /home/
COPY --from=q-server-build /lib/ /lib/
COPY --from=q-server-build /lib64/ /lib64/
COPY --from=q-server-build /usr/ /usr/

COPY --from=ton-node / /

ADD q-server /q-server
ADD ton-node /ton-node
ADD Procfile /honcho-cfg/
ADD arango /arango

RUN apk --update add nginx dos2unix bash git py-pip; \
    addgroup --system --gid 103 nginx; \ 
    adduser --system --disabled-login --ingroup nginx --no-create-home --home /nonexistent --gecos "nginx user" --shell /bin/false --uid 103 nginx; \
    mkdir -p /run/nginx; \
    rm -rf /etc/nginx/conf.d/; \
    rm -rf /etc/nginx/v.host/; \
    rm -f /etc/nginx/nginx.conf; \
    pip install --upgrade pip; \
    pip install -Iv honcho==1.0.1; \
    mkdir -p '/var/lib/arangodb3'; \
    mkdir -p '/var/lib/arangodb3-apps'; \
    chmod +w '/var/lib/arangodb3'; \
    chmod +w '/var/lib/arangodb3-apps'

RUN dos2unix /q-server/entrypoint /arango/entrypoint /arango/config /ton-node/entrypoint /honcho-cfg/Procfile

RUN chmod +x /q-server/entrypoint; \
    chmod +x /arango/entrypoint; \
    chmod +x /ton-node/entrypoint

WORKDIR /honcho-cfg
RUN /arango/entrypoint init
COPY nginx.conf.d /etc/nginx/nginx.conf
COPY ton-live/web /var/www

EXPOSE 80
EXPOSE 3000 

ENTRYPOINT ["honcho"]
CMD ["start"]
