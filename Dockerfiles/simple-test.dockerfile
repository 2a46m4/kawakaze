# Simple test image for FreeBSD
# Minimal image to test basic Dockerfile functionality

FROM freebsd-15.0-release

LABEL maintainer="kawakaze@example.com"
LABEL description="Simple test image for FreeBSD"
LABEL version="1.0"

# Set environment variables
ENV TEST_VAR="hello"
ENV TEST_PATH="/test"

# Set working directory
WORKDIR /test

# Create a test directory structure
RUN mkdir -p /test/dir1 /test/dir2 && \
    echo "This is a test file" > /test/test.txt && \
    echo "Another test file" > /test/dir1/file1.txt

# Create a simple script
RUN echo '#!/bin/sh' > /test/hello.sh && \
    echo 'echo "Hello from Kawakaze!"' >> /test/hello.sh && \
    echo 'echo "TEST_VAR=${TEST_VAR}"' >> /test/hello.sh && \
    chmod +x /test/hello.sh

# Create a test user
RUN pw useradd testuser -d /test -s /bin/sh && \
    chown -R testuser:wheel /test

# Expose a test port (even though we're not running a server)
EXPOSE 8080

# Set a volume for testing
VOLUME ["/test/data"]

# Switch to test user
USER testuser

# Default command - run the hello script
CMD ["/test/hello.sh"]
