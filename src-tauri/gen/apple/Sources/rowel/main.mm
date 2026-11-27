#include "bindings/bindings.h"

// CredentialExchangeProbe.swift
extern "C" void rowel_credential_exchange_probe_install(void);

int main(int argc, char * argv[]) {
	rowel_credential_exchange_probe_install();
	ffi::start_app();
	return 0;
}
