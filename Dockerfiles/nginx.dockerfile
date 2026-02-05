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
RUN pkg update && pkg install -y nginx && rm -rf /var/cache/pkg

# Create a simple HTML page
RUN mkdir -p ${WEB_ROOT} && echo "<html><body><h1>Welcome to Kawakaze Nginx</h1><p>Running on FreeBSD!</p></body></html>" > ${WEB_ROOT}/index.html

# Configure nginx to run in foreground
# Create a minimal nginx configuration
RUN echo 'worker_processes  1; events { worker_connections  1024; } http { include       mime.types; default_type  application/octet-stream; sendfile        on; keepalive_timeout  65; server { listen       80; server_name  localhost; location / { root   ${WEB_ROOT}; index  index.html; } } }' > /usr/local/etc/nginx/nginx.conf

# Expose HTTP port
EXPOSE 80

# Set working directory
WORKDIR ${WEB_ROOT}

# Run nginx in foreground
CMD ["nginx", "-g", "daemon off;"]
