# VPN Control

Manage VPN proxy lifecycle via the `vpn_control` tool. Uses Clash as the proxy runtime with subscription-based node management.

## Key Concept

`vpn_control` manages a local Clash SOCKS5 proxy. It fetches proxy nodes from a subscription URL, selects the fastest node via health checks, and routes outbound traffic through the proxy. Use it when API calls or web requests fail due to network restrictions or geo-blocking.

## Workflow

1. **Check status** first: `vpn_control action="status"` — see if VPN is already active
2. **Enable** if needed: `vpn_control action="enable"` — fetches subscription, starts Clash, selects fastest node
3. **List nodes** to inspect: `vpn_control action="list_nodes"` — shows all nodes with health/latency
4. **Switch node** if current is slow: `vpn_control action="switch_node" node_name="<name>"`
5. **Refresh** subscription: `vpn_control action="refresh"` — re-fetch nodes and re-select best
6. **Disable** when done: `vpn_control action="disable"` — stops Clash, removes proxy

## vpn_control Actions

```
vpn_control action="status"                          # check if VPN is active, current node, latency
vpn_control action="enable"                          # start Clash proxy, auto-select fastest node
vpn_control action="disable"                         # stop Clash proxy
vpn_control action="list_nodes"                      # list all proxy nodes with health status
vpn_control action="switch_node" node_name="Tokyo"   # switch to a specific node
vpn_control action="refresh"                         # re-fetch subscription, re-select best node
vpn_control action="add_bypass" domain="example.com" # skip proxy for this domain
vpn_control action="remove_bypass" domain="example.com"
```

## When to Enable VPN

- API calls return connection timeout or network unreachable errors
- Requests to external services (GitHub, npm, pip, Docker Hub) are blocked or throttled
- User explicitly asks to use VPN or proxy
- Services behind a firewall or geo-restricted need to be accessed

## When to Disable VPN

- User asks to disable or stop VPN
- All network-dependent tasks are complete
- VPN is causing latency issues for local/domestic services

## Bypass List

Some domains should skip the proxy (local services, domestic APIs). Use `add_bypass` / `remove_bypass` to manage:

- Add bypass for domains that are faster without proxy (e.g. local mirrors, intranet)
- Remove bypass if a previously bypassed domain needs proxy access

## Rules

- ALWAYS check `status` before enabling — avoid double-enable errors.
- After `enable`, verify connectivity by checking the status output for active node and latency.
- If health shows high latency or unhealthy nodes, use `refresh` to re-fetch and re-select.
- If a specific node is needed, use `list_nodes` first to get exact node names, then `switch_node`.
- Do NOT leave VPN enabled indefinitely — disable when the network-restricted task is complete.
