#include <cstdlib>
#include <iostream>
#include <memory>
#include <string>

#include <grpcpp/ext/proto_server_reflection_plugin.h>
#include <grpcpp/grpcpp.h>
#include <grpcpp/health_check_service_interface.h>

#include "absl/log/initialize.h"
#include "sfu_service_impl.h"

std::string listenAddrFromEnv() {
    if (const char* e = std::getenv("SFU_GRPC_ADDR")) {
        return std::string(e);
    }
    return "0.0.0.0:50051";
}

void runServer(const std::string& listen_addr) {
    SfuServiceImpl service;

    grpc::EnableDefaultHealthCheckService(true);
    grpc::reflection::InitProtoReflectionServerBuilderPlugin();

    grpc::ServerBuilder builder;
    builder.AddListeningPort(listen_addr, grpc::InsecureServerCredentials());
    builder.RegisterService(&service);

    std::unique_ptr<grpc::Server> server(builder.BuildAndStart());

    if (!server) {
        std::cerr << "BuildAndStart failed for " << listen_addr << "\n";
        std::exit(1);
    }

    std::cerr << "SFU gRPC listening on " << listen_addr << "\n";
    server->Wait();
}

int main(int argc, char* argv[]) {
    absl::InitializeLog();

    runServer(listenAddrFromEnv());
    return 0;
}