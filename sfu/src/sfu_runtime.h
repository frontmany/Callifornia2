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
#include <utility>
#include <vector>

class SfuRuntime {
public:
    explicit SfuRuntime(std::shared_ptr<EventRouter> eventRouter);

    bool addPeer(const std::string& roomId,
                 const std::string& participantId,
                 const std::string& signalingId,
                 std::string& errorMessage);

    bool removePeer(const std::string& roomId, const std::string& participantId);

    // Remove all peers registered under signalingId.
    // Called when the SubscribeEvents stream for that signalingId is cancelled.
    // Returns the number of peers removed.
    int removePeersBySignaling(const std::string& signalingId);

    std::shared_ptr<EventRouter> eventRouter() const { return m_eventRouter; }

    std::shared_ptr<PeerSession> findPeerSession(const std::string& roomId,
                                                  const std::string& participantId) const;

private:
    std::shared_ptr<Room> getOrCreateRoom(const std::string& roomId);

    mutable std::mutex m_mutex;
    std::unordered_map<std::string, std::shared_ptr<Room>> m_rooms;

    // Index: signalingId -> list of (roomId, participantId) it owns.
    std::unordered_map<std::string,
                       std::vector<std::pair<std::string, std::string>>>
        m_signalingPeers;

    std::shared_ptr<EventRouter> m_eventRouter;
    rtc::Configuration m_rtcConfig;
};
