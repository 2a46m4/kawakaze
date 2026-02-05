# Redis cache server image for FreeBSD
# Builds a Redis cache server on FreeBSD

FROM freebsd-15.0-release

LABEL maintainer="kawakaze@example.com"
LABEL description="Redis cache server on FreeBSD"
LABEL version="1.0"

# Set environment variables
ENV REDIS_VERSION="7.2"
ENV REDIS_PORT="6379"

# Install Redis
RUN pkg update && \
    pkg install -y redis && \
    rm -rf /var/cache/pkg

# Create Redis working directory
RUN mkdir -p /var/db/redis && \
    chown -R redis:redis /var/db/redis

# Configure Redis to bind to all interfaces (for testing)
RUN sed -i '' 's/^bind 127.0.0.1/bind 0.0.0.0/' /usr/local/etc/redis.conf

# Expose Redis port
EXPOSE 6379

# Set working directory
WORKDIR /var/db/redis

# Set volume for data persistence
VOLUME ["/var/db/redis"]

# Start Redis server
CMD ["redis-server", "/usr/local/etc/redis.conf"]
