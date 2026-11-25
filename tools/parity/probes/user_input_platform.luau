-- UserInputService:GetPlatform on a playing client and server (script identity 2).
local UIS = game:GetService("UserInputService")

local function Info(Fn)
	local S, L, N, A, V = debug.info(Fn, "slna")
	return tostring(S) .. "|" .. tostring(L) .. "|" .. tostring(N) .. "|" .. tostring(A) .. "|" .. tostring(V)
end

local Fn = UIS.GetPlatform
print("uis GetPlatform " .. typeof(Fn) .. " " .. Info(Fn) .. " same=" .. tostring(rawequal(Fn, UIS.GetPlatform)))
print("uis call " .. P.try(function() return Fn(UIS) end))
print("uis arg " .. P.try(function() return Fn(UIS, 1) end))
print("uis other " .. P.try(function() return Fn(workspace) end))
print("uis none " .. P.try(function() return Fn() end))
print("uis lower-read " .. P.try(function()
	local Lower = (UIS :: any).getPlatform
	return typeof(Lower), rawequal(Lower, Fn)
end))
print("uis lower-call " .. P.try(function() return (UIS :: any):getPlatform() end))
