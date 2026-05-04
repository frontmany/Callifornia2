#pragma once

#include "event_router.h"
#include "ice_config.h"
#include "peer_session.h"
#include "room.h"

#include "signaling.pb.h"

#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <unordered_map>

class SfuRuntime {
public:
    explicit SfuRuntime(std::shared_ptr<EventRouter> eventRouter);

    bool addPeer(const std::string& roomId,
                 const std::string& participantId,
                 const std::string& signalingId,
                 std::string& errorMessage);

    bool removePeer(const std::string& roomId, const std::string& participantId);

    std::size_t activePeerCount() const;
    std::size_t activeRoomCount() const;

    bool sendEvent(const std::string& roomId,
                   const std::string& participantId,
                   const sfu::SFUEvent& event) const;

    std::shared_ptr<EventRouter> eventRouter() const { return m_eventRouter; }

    std::optional<std::string> findSignalingId(const std::string& roomId,
                                                const std::string& participantId) const;
    std::shared_ptr<PeerSession> findPeerSession(const std::string& roomId,
                                                  const std::string& participantId) const;

private:
    std::shared_ptr<Room> getOrCreateRoom(const std::string& roomId);

    mutable std::mutex m_mutex;
    std::unordered_map<std::string, std::shared_ptr<Room>> m_rooms;
    std::shared_ptr<EventRouter> m_eventRouter;
    rtc::Configuration m_rtcConfig;
};
