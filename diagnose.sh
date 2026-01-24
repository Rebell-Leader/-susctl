#!/bin/bash
git status > status.txt 2>&1
echo "--- LOG ---" >> status.txt
git log -1 >> status.txt 2>&1
echo "--- REBASE DIR ---" >> status.txt
ls -d .git/rebase* >> status.txt 2>&1
