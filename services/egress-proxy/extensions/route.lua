-- Turn the tunnel decision into a route decision.
--
-- The filter before this one asks the control API about a CONNECT and writes the
-- answer into dynamic metadata. It cannot change the route itself: after its
-- asynchronous call the header map of that filter is "continued", and Envoy only
-- clears the route cache for a mutation that happens before that point — which is
-- why `clearRouteCache` was tried and is not part of the Lua API in Envoy 1.31
-- (calling it is a runtime error, and the CONNECT then stayed on the tunnel
-- route, so nothing was ever inspected).
--
-- This filter has no asynchronous call. Its mutation happens while the headers
-- are still continuable, so Envoy clears the route cache and the router picks the
-- inspection route (or the plain tunnel) for the request as it is now, not as it
-- arrived.
--
-- Nothing is written when the previous filter did not ask a question (a plain
-- request, not a CONNECT), so an ordinary request keeps the route it matched.

function envoy_on_request(handle)
  local metadata = handle:streamInfo():dynamicMetadata():get("nestbox_route")
  if metadata == nil or metadata.inspect == nil then
    return
  end
  handle:headers():replace("x-nestbox-inspect", tostring(metadata.inspect))
end
