# Nginx web server image for FreeBSD
# Builds a simple nginx web server on FreeBSD

FROM freebsd-15.0-release

LABEL maintainer="kawakaze@example.com"
LABEL description="Nginx web server on FreeBSD"
LABEL version="1.0"

# Set environment variables
ENV NGINX_VERSION="1.26"
ENV WEB_ROOT="/usr/local/www/nginx"

# Install nginx
RUN pkg update && \
    pkg install -y nginx && \
    rm -rf /var/cache/pkg

# Create a simple HTML page
RUN mkdir -p ${WEB_ROOT} && \
    echo "<html><body><h1>Welcome to Kawakaze Nginx</h1><p>Running on FreeBSD!</p></body></html>" > ${WEB_ROOT}/index.html

# Configure nginx to run in foreground
# Create a minimal nginx configuration
RUN echo "worker_processes  1;" > /usr/local/etc/nginx/nginx.conf && \
    echo "events {" >> /usr/local/etc/nginx/nginx.conf && \
    echo "    worker_connections  1024;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "}" >> /usr/local/etc/nginx/nginx.conf && \
    echo "http {" >> /usr/local/etc/nginx/nginx.conf && \
    echo "    include       mime.types;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "    default_type  application/octet-stream;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "    sendfile        on;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "    keepalive_timeout  65;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "    server {" >> /usr/local/etc/nginx/nginx.conf && \
    echo "        listen       80;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "        server_name  localhost;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "        location / {" >> /usr/local/etc/nginx/nginx.conf && \
    echo "            root   ${WEB_ROOT};" >> /usr/local/etc/nginx/nginx.conf && \
    echo "            index  index.html;" >> /usr/local/etc/nginx/nginx.conf && \
    echo "        }" >> /usr/local/etc/nginx/nginx.conf && \
    echo "    }" >> /usr/local/etc/nginx/nginx.conf && \
    echo "}" >> /usr/local/etc/nginx/nginx.conf

# Expose HTTP port
EXPOSE 80

# Set working directory
WORKDIR ${WEB_ROOT}

# Run nginx in foreground
CMD ["nginx", "-g", "daemon off;"]
