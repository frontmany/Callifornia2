#pragma once

#include <atomic>
#include <memory>
#include <mutex>
#include <future>
#include <optional>
#include <string>
#include <functional>
#include <unordered_map>

#include <rtc/rtc.hpp>
#include "signaling.pb.h"
 
class EventRouter;

// One WebRTC PeerConnection to a client; Stage B: offer/answer + trickle ICE, no inter-peer forward.
class PeerSession : public std::enable_shared_from_this<PeerSession> {
public:
    using TrackAddedHandler = std::function<void(const std::string& publisherId,
                                                 const std::shared_ptr<rtc::Track>& track,
                                                 const std::string& trackId,
                                                 const std::string& kind,
                                                 const std::string& sourceType)>;
    using TrackRemovedHandler = std::function<void(const std::string& publisherId,
                                                   const std::string& trackId,
                                                   const std::string& kind,
                                                   const std::string& reason)>;

    PeerSession(std::string roomId, std::string participantId, std::string signalingId);
    ~PeerSession();

    PeerSession(const PeerSession&) = delete;
    PeerSession& operator=(const PeerSession&) = delete;

    const std::string& participantId() const;
    const std::string& roomId() const;
    const std::string& signalingId() const;

    void initPeerConnection(const std::shared_ptr<EventRouter>& router, rtc::Configuration config);
    void shutdownPeerConnection();

    bool hasPeerConnection() const;

    // Client offer -> local answer SDP (waits for onLocalDescription). Nullopt on error/timeout.
    std::optional<std::string> applyRemoteOffer(const std::string& sdp, std::string& errorMessage);
    bool addRemoteIceCandidate(const std::string& candidate, const std::string& sdpMid);
    void setTrackHandlers(TrackAddedHandler added, TrackRemovedHandler removed);
    std::shared_ptr<rtc::Track> addRelayTrackIfMissing(const std::string& relayTrackId,
                                                        const rtc::Description::Media& sourceDescription,
                                                        std::string& errorMessage);
    void removeRelayTrack(const std::string& relayTrackId);

private:
    void onLocalDescription(rtc::Description description);
    void onLocalCandidate(rtc::Candidate candidate);
    void onStateChange(rtc::PeerConnection::State state);
    void onIceStateChange(rtc::PeerConnection::IceState state);
    void onTrack(const std::shared_ptr<rtc::Track>& track);

    void emitIceCandidate(const rtc::Candidate& candidate);
    void emitPeerConnected();
    void emitPeerDisconnected(const std::string& reason);
    void emitTrackAdded(const std::string& trackId, const std::string& kind, const std::string& sourceType);
    void emitTrackRemoved(const std::string& trackId, const std::string& kind, const std::string& reason);
    void routeEventOrLog(const std::shared_ptr<EventRouter>& router,
                         const sfu::SFUEvent& event,
                         const char* eventName) const;
    static bool validateFixedSlotsOffer(const std::string& sdp, std::string& errorMessage);
    static std::string sourceTypeFromMid(std::string mid);

    std::string m_roomId;
    std::string m_participantId;
    std::string m_signalingId;
    std::shared_ptr<EventRouter> m_eventRouter;

    mutable std::mutex m_pcMutex;
    std::shared_ptr<rtc::PeerConnection> m_pc;
    std::shared_ptr<std::promise<std::string>> m_answerPromise;
    std::atomic<bool> m_negotiationInProgress{false};
    std::atomic<bool> m_peerConnectedEmitted{false};
    std::unordered_map<std::string, std::string> m_trackKindsById;
    std::unordered_map<std::string, std::shared_ptr<rtc::Track>> m_relayTracks;
    mutable std::mutex m_tracksMutex;
    TrackAddedHandler m_trackAddedHandler;
    TrackRemovedHandler m_trackRemovedHandler;
};
