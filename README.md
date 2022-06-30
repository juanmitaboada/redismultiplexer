# redismultiplexer

This is a program to transfer data from queues (push/pop) between different Redis server

## Description

RedisMultiplexer will take care of your queues in Redis. The purpose is to move packages from a source queue to a target queue that may be or not be in the same server and port.

You can configure as many targets as "clients" as you would like, the system works in 2 modes (replicant and spreader):

- `replicant mode` replicates the incoming packages from the source server to all clients or destinations
- `spreader mode` will send a package to each target at a time, using a Round-Robin target selection

RedisMultiplexer will take care of your servers by checking the size of the destination queues to avoid overloading, you can control all of that with the \*limit options in your configuration, also you can filter what data is delivered where with filter\* options, finally you can reorder the incoming queue with the ordering\* options:

General configuration:

- Explained soon!

Limits are optional:

- `timelimit`: the size of the queue will be checked every n-seconds
- `checklimit`: the size of the queue will be checked every n-packages
- `hardlimit`: the software will `stop` sending packages to the destination until the queue is freed and `softlimit` is reached
- `softlimit`: the software will `continue` sending packages to the destination after a `hardlimit` was detected and the queue is freed until being under `softlimit`
- `deleteblock`: when `hardlimit` is reached the software will delete n-oldests-packages from the queue as many times until the size of the queue is under `hardlimit`

Filters are optional:

- `filter`: this is a Regular Expression that will be matched with the header of the package
- `filter_until`: the header of the package will be as long until this substring is reached (the minimum between filter\_until and filter\_limit will be used)
- `filter_limit`: the header of the package will be as long until these total bytes is reached (the minimum between filter\_until and filter\_limit will be used)
- `filter_replace`: if this filter option is defined the Regular Expression will be replaced with this string (which may contain $X groups from Regex)

Ordering is optional:

- This feature is not yet implemented

## How all of this works

### Example 1: forwarding packages between server

Explanation soon!

### Example 2: load balancing queues between several servers

Explanation soon!

### Example 3: replicating queues into several servers

Explanation soon!

### Example 4: retention system for network outages

Explanation soon!

## Full example configuration

```yaml
name        : "Source"
hostname    : "127.0.0.1"
port        : 6379
password    : "abcdefghijklmnopqrstuvwxyz"
channel     : "SourceQueue"
children    : 2
mode        : "replicant"       # <--- choose between: "replicant" and "spreader"
# pid         : "config.pid"    <--- not available
# filter      : "ola"           <--- optional
# filter_until: "r"             <--- optional
# filter_limit: 11              <--- optional
# filter_replace: "OLA"         <--- optional
# ordering_buffer_time: 5       <--- not available
# ordering_limit: 200           <--- not available
# ordering_prets: '"ts":'       <--- not available
# ordering_posts: ','           <--- not available

clients:
  - name        : "Target 1"
    hostname    : "127.0.0.1"
    port        : 6379
    password    : "abcdefghijklmnopqrstuvwxyz"
    channel     : "TargetQueue1"
    timelimit   : 5             # <--- optional
    checklimit  : 100           # <--- optional
    softlimit   : 400           # <--- optional
    hardlimit   : 410           # <--- optional
    deleteblock : 100           # <--- optional
    # filter      : "ola"         <--- optional
    # filter_until: "r"           <--- optional
    # filter_limit: 11            <--- optional
    # filter_replace: "OLA"       <--- optional
    # filter      : "^(1|3)"      <--- optional
    # filter_until: "#"           <--- optional
    # filter_limit: 1             <--- optional
    # filter_replace: ""          <--- optional
  - name        : "DB2"
    hostname    : "127.0.0.1"
    port        : 6379
    password    : "abcdefghijklmnopqrstuvwxyz"
    channel     : "TargetQueue2"
    timelimit   : 5
    checklimit  : 100
    softlimit   : 400
    hardlimit   : 410
```
