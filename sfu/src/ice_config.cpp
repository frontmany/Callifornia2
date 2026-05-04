#include "ice_config.h"

#include <cstdlib>
#include <string>
#include <string_view>

namespace {

std::string_view trim(std::string_view s) {
    while (!s.empty() && (s.front() == ' ' || s.front() == '\t')) {
        s.remove_prefix(1);
    }
    while (!s.empty() && (s.back() == ' ' || s.back() == '\t')) {
        s.remove_suffix(1);
    }
    return s;
}

}  // namespace

rtc::Configuration loadIceConfiguration() {
    rtc::Configuration config;
    config.forceMediaTransport = true;

    const char* env = std::getenv("SFU_ICE_SERVERS");
    if (env == nullptr || env[0] == '\0') {
        config.iceServers.emplace_back("stun:stun.l.google.com:19302");
        return config;
    }

    std::string_view rest(env);
    while (!rest.empty()) {
        std::size_t comma = rest.find(',');
        std::string_view token = trim(rest.substr(0, comma));
        if (!token.empty()) {
            config.iceServers.emplace_back(std::string(token));
        }
        if (comma == std::string_view::npos) {
            break;
        }
        rest.remove_prefix(comma + 1);
    }

    if (config.iceServers.empty()) {
        config.iceServers.emplace_back("stun:stun.l.google.com:19302");
    }
    return config;
}
