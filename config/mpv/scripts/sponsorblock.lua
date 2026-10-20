-- Skip only paid sponsor segments. Failure leaves playback untouched.
-- API: https://wiki.sponsor.ajay.app/w/API_Docs
local mp = require 'mp'
local utils = require 'mp.utils'
local segments = {}
local generation = 0
local pending = nil

local function reset()
    generation = generation + 1
    segments = {}
    if pending then mp.abort_async_command(pending); pending = nil end
end

mp.register_event('start-file', reset)
mp.register_event('end-file', reset)
mp.register_event('file-loaded', function()
    local path = mp.get_property('path', '')
    local id = path:match('^https://www%.youtube%.com/watch%?v=([%w_-]+)$')
    if not id or #id ~= 11 then return end
    local current = generation
    pending = mp.command_native_async({
        name = 'subprocess', playback_only = true, capture_stdout = true, capture_stderr = true,
        args = {'curl', '--silent', '--show-error', '--fail', '--max-time', '8', '--get',
            '--data-urlencode', 'videoID=' .. id,
            '--data-urlencode', 'categories=["sponsor"]',
            '--data-urlencode', 'actionTypes=["skip"]',
            'https://sponsor.ajay.app/api/skipSegments'},
    }, function(success, result)
        if current ~= generation then return end
        pending = nil
        if not success or not result or result.status ~= 0 then return end
        local data = utils.parse_json(result.stdout)
        if type(data) ~= 'table' then return end
        for _, item in ipairs(data) do
            local span = item.segment
            if item.category == 'sponsor' and item.actionType == 'skip'
                and type(span) == 'table' and type(span[1]) == 'number'
                and type(span[2]) == 'number' and span[1] >= 0 and span[2] > span[1] then
                table.insert(segments, span)
            end
        end
    end)
end)

mp.observe_property('time-pos', 'number', function(_, position)
    if not position then return end
    for _, span in ipairs(segments) do
        if position >= span[1] and position < span[2] then
            mp.commandv('seek', tostring(span[2]), 'absolute+exact')
            mp.osd_message('Skipped sponsor', 2)
            return
        end
    end
end)
