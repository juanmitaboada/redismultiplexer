# redismultiplexer

This is a program to transfer data from queues (push/pop) between different Redis server

## Description

RedisMultiplexer will take care of your queues in Redis. The purpose is to move packages from a source queue to a target queue that may be or not be in the same server and port.

You can configure as many targets as "clients" as you would like, the system works in 2 modes (replicant and spreader):

- `replicant mode` replicates the incoming packages from the source server to all clients or destinations
- `spreader mode` will send a package to each target at a time, using a Round-Robin target selection
RedisMultiplexer will take care of your servers by checking the size of the destination queues to avoid overloading, you can control all of that with the \*limit options in your configuration, also you can filter what data is delivered where with filter\* options, finally you can reorder the incoming queue with the ordering\* options:

## Configuration

Every node or connection to Redis will shared the same `General Configuration` and optional `Filters`.

The configuration has 2 defined zones, the first one is the source server which will have all its fields on the root of the YAML configuration while the target or destination servers will be under the entry `clients`:

`Root` of the YAML configuration belongs to source server and contains:
- `General configuration`: explained below
- Optional `Filters`: explained below
- Optional `Ordering`: explained below
- `pid`: pid file of the executing RedisMultiplexer
- `children`: total of threads or workers to be started for processing (usually 2 is enought)
- `mode`: there 2 working modes: replicant and spreader, explained before

Inside the `clients` entry of the YAML configuration:
- `General configuration`: explained below
- Optional `Limits`: explained below
- Optional `Filters`: explained below

### General configuration:

Every configuration will have

- `name`: will be used on the screen when RedisMultiplexer has to show some message on the screen
- `hostname`: hostname of the Redis server
- `post`: post number of the Redis server
- `password`: password of the Redis server
- `channel`: name of queue to process

### Filters are optional:

- `filter`: this is a Regular Expression that will be matched with the header of the package
- `filter_until`: the header of the package will be as long until this substring is reached (the minimum between filter\_until and filter\_limit will be used)
- `filter_limit`: the header of the package will be as long until these total bytes is reached (the minimum between filter\_until and filter\_limit will be used)
- `filter_replace`: if this filter option is defined the Regular Expression will be replaced with this string (which may contain $X groups from Regex)

As an example:
- For the incoming string "Hello world"
- We would like the filter to stop analizying after 11 bytes or "r"
- We would like to filter "ell"
- We would like the filtered string being replaced with ELL, so the result would e "HELLo world"
- We would use the configuration:
```yaml
filter: "ell"
filter_until: "r"
filter_limit: 11
filter_replace: "ELL"
```

### Limits are optional:

- `timelimit`: the size of the queue will be checked every n-seconds
- `checklimit`: the size of the queue will be checked every n-packages
- `hardlimit`: the software will `stop` sending packages to the destination until the queue is freed and `softlimit` is reached
- `softlimit`: the software will `continue` sending packages to the destination after a `hardlimit` was detected and the queue is freed until being under `softlimit`
- `deleteblock`: when `hardlimit` is reached the software will delete n-oldests-packages from the queue as many times until the size of the queue is under `hardlimit`

### Ordering is optional:

When several servers are dumping their information to the same queue the packages maybe disordered since the casuality of the real-time procesing. Let's imagine that you need to process this data by a Machine Learning system and because it is unordered the ML will may learn the future of its actions. It would be very easy to avoid this problem if the Queue gets ordered before being processed.

This feature take care of ordering the incoming queue before being processed, it keeps a timed buffer of few seconds and use a prefix and postfix to find a string that can be parsed to be used for comparing with other packages. RedisMultiplexer will extract the first `ordering_limit` bytes from the package and it will look for `ordering` Regular Expression matching the named group "ts" as the timestamp used for ordering the list of packages. The ts string extracted will be parsed as u128:

- `ordering`: what is the Regular Expression used to parse the times substring (matching with group 'ts' will be required in your Regex expression. Example:
```yaml
ordering: '.*"ts": *(?P<ts>\d+),.*#'
``` 
- `ordering_buffer_time`: total of seconds that new packages must stay in the buffer so they get ordered
- `ordering_limit`: how many bytes to process during the extraction of the substring

As an example:
- For the package: '{"a": "abc", "ts": 12345678, "b": 88}'
- Where "ts" is the key holding the timed string, example:
```yaml
ordering: '.*"ts": *(?P<ts>\d+),.*#'
ordering_buffer_time: 30
ordering_limit: 200
```

## How all of this works

### Example 1: forwarding packages between server

RedisMultiplexer can be used as a forwarder between server. This is specially good to transfer packages between servers and avoid overloading of the source server. It is a very good idea to set RedisMultiplexer on the source side of the connection so if there is some network delays or outages, the system will be dropping the packages on the source avoiding problems derived from source queue being overloaded.

### Example 2: load balancing queues between several servers

RedisMultiplexer may be used as a load balancer. Using the "spreader" mode you can put packages into the source queue and RedisMultiplexer will spread all the packages to all the target servers or `clients`. When some of the servers get overloaded RedisMultiplexer will avoid it for a while until its queue is freed.

### Example 3: replicating queues into several servers

RedisMultiplexer may be used as a replicant system. Using the "replicant" mode you can put packages into the source queue and RedisMultiplexer will send a copy of it to all to all the target servers or `clients`. When some of the servers get overloaded RedisMultiplexer will avoid it for a while until its queue is freed.


### Example 4: retention system for network outages

One of the interesting uses of RedisMultiplexer is as a retention system for network delays and network outages. This is very good when it is used on the source side of the connection, because it will keep the local queue empty if there are network problems or overloaded queues on the remote side of the connection.

### Example 5: multiple RedisMultiplexer services

It is a common use to set up several RedisMultiplexer in the same server. Les's imagine you would like to teach several ML systems on remote servers and each of them will be teached with specific data depending on the incoming packages.

Most probably the best approach would be to set up a RedisMultiplexer in `replicant` mode to split in the same way to all clients and filter the data in several queues (as many as different ML systems) we will have, the filtering system per client will help you to decide what data is sent to what queue. You may use ordering system as well to keep order of the incoming data.

Now let's imagine that one of the queues will be used to teach the same ML with different setups, so then you would use a RedisMultiplexer with the `replicant` mode to send data to both ML systems.

But let's go farther, you will need a test data and a learning data in disjoint groups so learning system won't learn from test data and we can use test data for prediction to test how good the ML system is learning. You can split data in percentages, lets say 10% test / 90% learning. You would use a RedisMultiplexer in `spreader` mode to 10 different clients, 1 of those clients will be the testing queue while the other 9 clients will be the same learning queue. Still you may would like another RedisMultiplexer as a forwarder to reorder data.

Each of this queues you just made are still in the same server, so you would use one RedisMultiplexer by each of those queues to put data out from that server to another remote server, in this way you would be using RedisMultiplexer as a rentention system in case there are network issues.

## Full example configuration

```yaml
name        : "Source"
hostname    : "127.0.0.1"
port        : 6379
password    : "abcdefghijklmnopqrstuvwxyz"
channel     : "SourceQueue"
children    : 2
mode        : "replicant"               # choose between: "replicant" and "spreader"
# pid         : "config.pid"            # optional
# filter      : "ola"                   # optional
# filter_until: "r"                     # optional
# filter_limit: 11                      # optional
# filter_replace: "OLA"                 # optional
# ordering: '.*"ts": *(?P<ts>\d+),.*#'  # optional
# ordering_buffer_time: 5               # optional
# ordering_limit: 200                   # optional

clients:
  - name        : "Target 1"
    hostname    : "127.0.0.1"
    port        : 6379
    password    : "abcdefghijklmnopqrstuvwxyz"
    channel     : "TargetQueue1"
    timelimit   : 5                     # optional
    checklimit  : 100                   # optional
    softlimit   : 400                   # optional
    hardlimit   : 410                   # optional
    deleteblock : 100                   # optional
    # filter      : "ola"               # optional
    # filter_until: "r"                 # optional
    # filter_limit: 11                  # optional
    # filter_replace: "OLA"             # optional
    # filter      : "^(1|3)"            # optional
    # filter_until: "#"                 # optional
    # filter_limit: 1                   # optional
    # filter_replace: ""                # optional
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
