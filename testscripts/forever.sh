#!/usr/bin/env bash

# OK, fine, it isn't really FOREVER, but whatever.
# Stay quiet about it and no one has to know.
# (I like this cause if the test does leak this process it won't be
# leaked forever, and sleep is very cheap & simple vs. e.g. a
# forever-loop with sleep in it)
sleep 100000
