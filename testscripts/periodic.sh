#!/usr/bin/env bash

# 100000 seconds -> ~28 hours.
# Should be fine for a test... presumably we won't ever be waiting that long.
for i in {0..100000}
do
		sleep 1
		printf "\n$i"
done
