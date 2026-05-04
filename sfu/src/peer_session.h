#pragma once

#include <atomic>
#include <memory>
#include <mutex>
#include <future>
#include <optional>
#include <string>

#include <rtc/rtc.hpp>
 
class EventRouter;

// One WebRTC PeerConnection to a client; Stage B: offer/answer + trickle ICE, no inter-peer forward.
class PeerSession : public std::enable_shared_from_this<PeerSession> {
public:
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
    std::optional<std::string> applyRemoteOffer(const std::string& sdp);
    bool addRemoteIceCandidate(const std::string& candidate, const std::string& sdpMid);

private:
    void onLocalDescription(rtc::Description description);
    void onLocalCandidate(rtc::Candidate candidate);
    void onStateChange(rtc::PeerConnection::State state);
    void onIceStateChange(rtc::PeerConnection::IceState state);

    void emitIceCandidate(const rtc::Candidate& candidate);
    void emitPeerConnected();
    void emitPeerDisconnected(const std::string& reason);

    std::string m_roomId;
    std::string m_participantId;
    std::string m_signalingId;
    std::shared_ptr<EventRouter> m_eventRouter;

    mutable std::mutex m_pcMutex;
    std::shared_ptr<rtc::PeerConnection> m_pc;
    std::shared_ptr<std::promise<std::string>> m_answerPromise;
    std::atomic<bool> m_peerConnectedEmitted{false};
};
