#include "bindings/bindings.h"

// CredentialExchange.swift
extern "C" void rowel_credential_exchange_install(void);

int main(int argc, char * argv[]) {
	rowel_credential_exchange_install();
	ffi::start_app();
	return 0;
}
