-- Run from the repo root: lua tests/sponsorblock_test.lua
local events, observers, callbacks, commands = {}, {}, {}, {}
local path = 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
local decoded, aborted
local mp = {
    register_event = function(name, fn) events[name] = fn end,
    observe_property = function(name, _, fn) observers[name] = fn end,
    get_property = function() return path end,
    command_native_async = function(command, callback)
        assert(command.name == 'subprocess' and command.args[1] == 'curl')
        callbacks[#callbacks + 1] = callback
        return #callbacks
    end,
    abort_async_command = function(id) aborted = id end,
    commandv = function(...) commands[#commands + 1] = {...} end,
    osd_message = function() end,
}
package.preload.mp = function() return mp end
package.preload['mp.utils'] = function() return {parse_json = function() return decoded end} end
dofile('config/mpv/scripts/sponsorblock.lua')

events['start-file']()
events['file-loaded']()
decoded = {
    {category = 'sponsor', actionType = 'skip', segment = {10, 20}},
    {category = 'intro', actionType = 'skip', segment = {30, 40}},
    {category = 'sponsor', actionType = 'mute', segment = {40, 50}},
    {category = 'sponsor', actionType = 'skip', segment = {50, 40}},
}
callbacks[1](true, {status = 0, stdout = 'fixture'})
observers['time-pos']('time-pos', 5)
assert(#commands == 0)
observers['time-pos']('time-pos', 10)
assert(#commands == 1 and commands[1][2] == '20' and commands[1][3] == 'absolute+exact')
observers['time-pos']('time-pos', 20)
observers['time-pos']('time-pos', 35)
observers['time-pos']('time-pos', 45)
assert(#commands == 1)

-- An in-flight response from a previous file cannot change the current segments.
events['file-loaded']()
events['end-file']()
assert(aborted == 2)
callbacks[2](true, {status = 0, stdout = 'stale'})
observers['time-pos']('time-pos', 15)
assert(#commands == 1)

-- Network failure leaves playback alone; local files make no request.
events['start-file']()
events['file-loaded']()
callbacks[3](false, nil)
observers['time-pos']('time-pos', 15)
assert(#commands == 1)
path = '/tmp/local.wav'
events['file-loaded']()
assert(#callbacks == 3)
print('SponsorBlock tests passed: skip boundaries, categories, stale responses, failure, local media.')
