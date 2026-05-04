#pragma once

#include <rtc/rtc.hpp>

// SFU_ICE_SERVERS: comma-separated ICE server URIs (stun:/turn:...). If unset, uses public Google STUN.
rtc::Configuration loadIceConfiguration();
