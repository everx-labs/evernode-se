events {
}
http {
    # q-server
    upstream q-server {
        ## Can be connected with "proxy_nw" network
        # q-server
        server 127.0.0.1:4000;
    }

    # ton-node
    upstream ton-node {
        # ton-node
        server 127.0.0.1:3000;
    } 

    server {
        listen 80 reuseport;
        location /topics/requests {
            proxy_http_version 1.1;
            proxy_pass http://ton-node;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_set_header Host $host;
            proxy_read_timeout 7200;
            proxy_buffering off;
            add_header 'Access-Control-Allow-Methods' 'GET, POST, PUT, DELETE, PATCH, OPTIONS';
            add_header 'Access-Control-Allow-Headers' 'X-Requested-With, Content-Type, Authorization';
            add_header 'Access-Control-Expose-Headers' 'X-Requested-With, Content-Type, Authorization';
            add_header 'Access-Control-Allow-Origin' '*';
        }
        location /graphql {
            proxy_http_version 1.1;
            proxy_pass http://q-server;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_set_header Host $host;
            proxy_read_timeout 7200;
            proxy_buffering off;
            add_header 'Access-Control-Allow-Methods' 'GET, POST, PUT, DELETE, PATCH, OPTIONS';
            add_header 'Access-Control-Allow-Headers' 'X-Requested-With, Content-Type, Authorization';
            add_header 'Access-Control-Expose-Headers' 'X-Requested-With, Content-Type, Authorization';
        }
        include /etc/nginx/mime.types;                                                                 
        types {                                                                                        
            application/wasm wasm;                                                                     
        }                                                                                              
        try_files $uri $uri/ /index.html;
        root /var/www;
    }
}
