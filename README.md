# agent-hacking
This repo shows 2 agent hack scenarios that are the most 
critical to the 2 ways that agents are utilized today.
The purpose of this repo is to run these scenarios in 
environments with proper tracing to see what they look like 
at that level, and then to add security to make these increasingly
difficult to pull off. 

The scenarios are:

1. web facing agents being convinced to break container in a
   deployed VM
2. local running agents writing a remote access trojan
   on a personal machine


In the case of the web facing agent, a user might be able to prompt the agent,
get it to break normal protocol, reveal information about it's environment,
and ultimately search the underlying VM for secrets, expose code, or simply 
allow the VM to be utilized for something totally different like crime or crypto mining.

In the case of the RAT, obviously a hacker would want to hide a jailbreak
on the internet, have someone's local agent read it, and create the ability
for the hacker to be able to control their computer remotely. 
