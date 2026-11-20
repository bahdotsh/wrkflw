#!/bin/bash
# The demo's tests.
set -e
source ./greet.sh
got=$(greet world)
[ "$got" = "Hello, world!" ] || { echo "FAIL: greet world printed '$got', want 'Hello, world!'"; exit 1; }
echo "PASS: greet world"
