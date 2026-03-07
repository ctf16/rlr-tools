# rlr-tools
Rocket League replay verification tools.
---

## Features

1. Replay Verifier
   - Parses the `.replay` binary
   - Hashes each node
   - Builds a hash tree
   - Signs the root
   - Stores verification in a `.sig` file
  
2. Bot detection
   - Detect anomalies in player input
     - Lots of rapid, alternating full-steer inputs in a short time
     - Repeated inputs from 0 to a precise input level 
