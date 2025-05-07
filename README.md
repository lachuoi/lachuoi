# LACHUOI  

A collection of very small microservices using WebAssembly (Wasm) and the Spin framework.

Eventually, each individual app in the apps directory will be moved to its own Git repository.
However, since I am currently the sole developer on this project, I will keep everything in one place for now.

Build Requirements
- [Spin from Spinframework](https://github.com/spinframework/spin)
- [Spin trigger cron as Spin plugin](https://github.com/spinframework/spin-trigger-cron)  
  Required if you use apps that run with a cron scheduler.  
  Install with:  
    `spin plugins install --url https://github.com/fermyon/spin-trigger-cron/releases/download/canary/trigger-cron.json`
- [Just](https://github.com/casey/just)  
---
Project LACHUOI is named after the Vietnamese word lá chuối, meaning "banana leaf."
It is licensed under the AGPL v3, unless otherwise noted in an app/service directory or its files.
