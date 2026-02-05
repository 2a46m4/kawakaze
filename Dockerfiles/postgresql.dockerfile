# PostgreSQL database server image for FreeBSD
# Builds a PostgreSQL database server on FreeBSD

FROM freebsd-15.0-release

LABEL maintainer="kawakaze@example.com"
LABEL description="PostgreSQL database server on FreeBSD"
LABEL version="1.0"

# Set environment variables
ENV PGDATA="/var/db/postgres/data"
ENV PGPORT="5432"

# Install PostgreSQL
RUN pkg update && \
    pkg install -y postgresql16-server && \
    rm -rf /var/cache/pkg

# Create PostgreSQL data directory
RUN mkdir -p ${PGDATA} && \
    chown -R postgres:postgres ${PGDATA} && \
    chmod 700 ${PGDATA}

# Expose PostgreSQL port
EXPOSE 5432

# Set working directory
WORKDIR /var/db/postgres

# Switch to postgres user and initialize the database
USER postgres
RUN initdb -D ${PGDATA}

# Configure PostgreSQL to accept connections
# Allow all IPv4 connections (for testing purposes)
RUN echo "host all all 0.0.0.0/0 md5" >> ${PGDATA}/pg_hba.conf && \
    echo "listen_addresses = '*'" >> ${PGDATA}/postgresql.conf

# Switch back to root for the entrypoint
USER root

# Create entrypoint script
RUN echo '#!/bin/sh' > /usr/local/bin/start-postgres && \
    echo 'if [ ! -d "${PGDATA}/base" ]; then' >> /usr/local/bin/start-postgres && \
    echo '    echo "Initializing PostgreSQL database..."' >> /usr/local/bin/start-postgres && \
    echo '    initdb -D ${PGDATA}' >> /usr/local/bin/start-postgres && \
    echo '    echo "host all all 0.0.0.0/0 md5" >> ${PGDATA}/pg_hba.conf' >> /usr/local/bin/start-postgres && \
    echo '    echo "listen_addresses = '*'" >> ${PGDATA}/postgresql.conf' >> /usr/local/bin/start-postgres && \
    echo 'fi' >> /usr/local/bin/start-postgres && \
    echo 'echo "Starting PostgreSQL..."' >> /usr/local/bin/start-postgres && \
    echo 'exec pg_ctl -D ${PGDATA} -l ${PGDATA}/logfile start' >> /usr/local/bin/start-postgres && \
    echo 'tail -f ${PGDATA}/logfile' >> /usr/local/bin/start-postgres && \
    chmod +x /usr/local/bin/start-postgres

# Set volume for data persistence
VOLUME [${PGDATA}]

# Start PostgreSQL
ENTRYPOINT ["/usr/local/bin/start-postgres"]
