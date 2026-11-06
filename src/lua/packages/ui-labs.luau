--!nocheck
-- Stand-in for the UI Labs utility package (pepeeltoro41/ui-labs 2.x) for places whose copy lives
-- only in Studio, adapted from its source (MIT, THIRD_PARTY_NOTICES.md). It runs as a ModuleScript
-- inside the emulated DataModel: the package's control tables, story creators and Environment API
-- as seen outside the UI Labs plugin (no environment injection: IsStory() is false).

local UserInputService = game:GetService("UserInputService")

local UILabs = {}

UILabs._version = "2.4.2"

-- Controls

local function BaseControl(ControlType: string, Default: any): { [string]: any }
	return {
		EntryType = "Control",
		Type = ControlType,
		ControlValue = Default,
	}
end

function UILabs.String(Default: string, Filters: { any }?)
	local Control = BaseControl("String", Default)
	Control.Filters = Filters
	return Control
end

function UILabs.Number(Default: number, Min: number?, Max: number?, Step: number?, Dragger: boolean?, Sensibility: number?)
	local Control = BaseControl("Number", Default)
	Control.Min = Min
	Control.Max = Max
	Control.Step = Step
	Control.Dragger = if Dragger == nil then true else Dragger
	Control.Sensibility = Sensibility or (Default * 10)
	return Control
end

function UILabs.Boolean(Default: boolean)
	return BaseControl("Boolean", Default)
end

function UILabs.Choose(List: { any }, DefaultIndex: number?)
	if #List <= 0 then
		error("UI-Labs: Array given in a Choose control is empty")
	end
	if DefaultIndex and DefaultIndex > #List then
		error(`UI-Labs: Def index ({DefaultIndex}) given for the array is outside of the array size ({#List})`)
	end

	local Control = BaseControl("Choose", List[DefaultIndex or 1])
	Control.List = List
	Control.DefIndex = DefaultIndex or 1
	return Control
end

function UILabs.EnumList(List: { [string]: any }, DefaultKey: string)
	if List[DefaultKey] == nil then
		error(`UI-Labs: Key given for the EnumList list ({DefaultKey}) does not exist in the list`)
	end

	local Control = BaseControl("EnumList", List[DefaultKey])
	Control.List = List
	Control.DefIndex = DefaultKey
	return Control
end

function UILabs.RGBA(Default: Color3, Transparency: number?)
	return BaseControl("RGBA", {
		Color = Default,
		Transparency = Transparency or 0,
	})
end

function UILabs.Slider(Default: number, Min: number, Max: number, Step: number?)
	if Max <= Min then
		error(`UI-Labs: Max slider value ({Max}) must be greater than the Min value ({Min})`)
	end

	local Control = BaseControl("Slider", Default)
	Control.Min = Min
	Control.Max = Max
	Control.Step = Step
	return Control
end

function UILabs.Object(ClassName: string?, Default: Instance?, Predicator: ((Instance) -> boolean)?)
	local Control = BaseControl("Object", Default)
	Control.ClassName = ClassName or "Instance"
	Control.Predicator = Predicator
	return Control
end

UILabs.Primitive = {
	string = UILabs.String,
	number = UILabs.Number,
	boolean = UILabs.Boolean,
}

UILabs.Advanced = {
	Choose = UILabs.Choose,
	EnumList = UILabs.EnumList,
	Object = UILabs.Object,
	RGBA = UILabs.RGBA,
	Slider = UILabs.Slider,
}

UILabs.Datatype = {
	Color3 = function(Default: Color3)
		return BaseControl("Color3", Color3.new(math.clamp(Default.R, 0, 1), math.clamp(Default.G, 0, 1), math.clamp(Default.B, 0, 1)))
	end,
}

function UILabs.ConvertControl(Control: any)
	local ControlType = typeof(Control)
	if ControlType == "table" then
		return Control
	end

	local Converter = UILabs.Primitive[ControlType] or UILabs.Datatype[ControlType]
	if not Converter then
		error(`UI-Labs: Control ({Control}) is not a valid control`)
	end
	return Converter(Control)
end

function UILabs.ControlGroup(Controls: { [string]: any })
	return {
		EntryType = "ControlGroup",
		Controls = Controls,
	}
end

function UILabs.Ordered(Control: any, Order: number)
	local Converted = UILabs.ConvertControl(Control)
	Converted.Order = Order
	return Converted
end

-- Control state helpers

function UILabs.ListenControl(Info: any, Callback: (any) -> ())
	if Info.__old ~= Info.__new then
		Callback(Info.__new)
	end
end

function UILabs.CreateControlStates(Converted: { [string]: any }, Controls: { [string]: any }, Creator: (any) -> any)
	local States = {}
	for Key, Control in pairs(Converted) do
		local Value = Controls[Key]
		if Control.EntryType == "ControlGroup" then
			States[Key] = UILabs.CreateControlStates(Control.Controls, Value, Creator)
			continue
		end
		States[Key] = Creator(Value)
	end
	return States
end

function UILabs.UpdateControlStates(States: { [string]: any }, Converted: { [string]: any }, Controls: { [string]: any }, Updater: (any, any) -> ())
	for Key, Control in pairs(Converted) do
		local Value = Controls[Key]
		if Control.EntryType == "ControlGroup" then
			UILabs.UpdateControlStates(States[Key], Control.Controls, Value, Updater)
			continue
		end
		Updater(States[Key], Value)
	end
end

-- Story creators

local function Combine(Story: { [string]: any }, Info: { [string]: any }): { [string]: any }
	for Key, Value in pairs(Info) do
		Story[Key] = Value
	end
	return Story
end

function UILabs.CreateRoactStory(Info: { [string]: any }, Render: any)
	return Combine({ use = "roact", story = Render }, Info)
end

function UILabs.CreateReactStory(Info: { [string]: any }, Render: any)
	return Combine({ use = "react", story = Render }, Info)
end

function UILabs.CreateFusionStory(Info: { [string]: any }, Render: any)
	return Combine({ use = "fusion", story = Render }, Info)
end

function UILabs.CreateIrisStory(Info: { [string]: any }, Render: any)
	return Combine({ use = "iris", story = Render }, Info)
end

function UILabs.CreateVideStory(Info: { [string]: any }, Render: any)
	return Combine({ use = "Vide", story = Render }, Info)
end

function UILabs.CreateGenericStory(Info: { [string]: any }, Render: any)
	return Combine({ render = Render }, Info)
end

-- Environment (outside the plugin: nothing is injected)

local Environment = {}

Environment.EnvGlobalInjectionKey = "__hotreload_env_global_injection__"

function Environment.GetEnvGlobalInjection()
	return nil
end

function Environment.IsStory()
	return false
end

Environment.Unmount = function() end
Environment.Reload = function() end
Environment.CreateSnapshot = function() end
Environment.SetStoryHolder = function() end
Environment.GetJanitor = function()
	return nil
end
Environment.InputListener = nil
Environment.UserInput = UserInputService
Environment.EnvironmentUID = ""
Environment.PreviewUID = ""
Environment.OriginalG = _G
Environment.PluginWidget = nil
Environment.Plugin = nil

UILabs.Environment = Environment

return UILabs
