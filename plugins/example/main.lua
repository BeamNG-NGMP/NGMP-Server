local M = {}

M.onPluginLoad = function()
    print("hi from da lua")

    print("plugins:")
    for k, v in pairs(ngmp.get_plugins()) do
        print(v)
    end

    -- while true do end

    print("okay bye !!!")
end

return M
