use blake3::Hasher;

fn hex_to_bytes(s: &str) -> Vec<u8> {
    hex::decode(s.trim()).expect("invalid hex")
}

fn main() {
    // Test vectors from the historical validation context.
    let test_vectors = [
        // Single signature test
        (
            "4176cf2ee56b3eed1e8da083851f41cae11532fc70a63ca1ca9f17bc9a4c2fd3dcdf60df1c1a57465f0d112995a6f289511c8e0a79c806fb79165544a439d11c0201",
            "20e1d5835e09f3c3dad209debcb7b3bf3fb0e0d9642471f5db36c9ea58338b06beac",
            "200749c89953b463d1e186a16a941f9354fa3fff313c391149e47961b95dd4df28ac",
        ),
        // Multi-signature test
        (
            "41ca6f8d104b47ca8ab133d98b3794b49f00ec5d2dce8253e78de035dfbc8f40a2fefa3086c3a181d9f1755a8f4ada4f8a4b8982b361853c8020009e1a752debce0141fdb58c2c25fcfe37d427967c34700f92e9eb1df0f2f9ff366444d92357ff35a270ee5445287031e4c0f72acda20876ccf918de1039a41e9b5f83b3737223f995014c875220ecdd9ec9f2c53ed8e5a170cc88354e133299022da55e1e8bd3c61d8b9dcbd7df2068f191b6aca3d9d8cfa2edb0c44a10fc87dc36b62e1d02228257ccdf979b1fce20b1503ef14aa6773ba3a1f012dbea2992e181766c35c5bc17465b5f57807540bf2006e161ced6b77c11b9a317080a899121a9c6df30a76490402f9a3b7e18bce97b54ae",
            "aa2071b6c2c604a8830a1484ba469e845c37bb0af32f044bc8fd0c892c8878419e8587",
            "206c376f9da440494e18b283803698ed13249af93be3e99f58f42d7d82744d3d15ac",
        ),
        // Last sig incorrect multi-signature test
        (
            "41ca6f8d104b47ca8ab133d98b3794b49f00ec5d2dce8253e78de035dfbc8f40a2fefa3086c3a181d9f1755a8f4ada4f8a4b8982b361853c8020009e1a752debce0141fdb58c2c25fcfe37d427967c34700f92e9eb1df0f2f9ff366444d92357ff3da270ee5445287031e4c0f72acda20876ccf918de1039a41e9b5f83b3737223f995014c875220ecdd9ec9f2c53ed8e5a170cc88354e133299022da55e1e8bd3c61d8b9dcbd7df2068f191b6aca3d9d8cfa2edb0c44a10fc87dc36b62e1d02228257ccdf979b1fce20b1503ef14aa6773ba3a1f012dbea2992e181766c35c5bc17465b5f57807540bf2006e161ced6b77c11b9a317080a899121a9c6df30a76490402f9a3b7e18bce97b54ae",
            "aa2071b6c2c604a8830a1484ba469e845c37bb0af32f044bc8fd0c892c8878419e8587",
            "206c376f9da440494e18b283803698ed13249af93be3e99f58f42d7d82744d3d15ac",
        ),
        // First sig incorrect multi-signature test
        (
            "41ca6f8d104b47ca8ab133d98b3794b49f00ec5d2dce8253e78de035dfbc8f41a2fefa3086c3a181d9f1755a8f4ada4f8a4b8982b361853c8020009e1a752debce0141fdb58c2c25fcfe37d427967c34700f92e9eb1df0f2f9ff366444d92357ff35a270ee5445287031e4c0f72acda20876ccf918de1039a41e9b5f83b3737223f995014c875220ecdd9ec9f2c53ed8e5a170cc88354e133299022da55e1e8bd3c61d8b9dcbd7df2068f191b6aca3d9d8cfa2edb0c44a10fc87dc36b62e1d02228257ccdf979b1fce20b1503ef14aa6773ba3a1f012dbea2992e181766c35c5bc17465b5f57807540bf2006e161ced6b77c11b9a317080a899121a9c6df30a76490402f9a3b7e18bce97b54ae",
            "aa2071b6c2c604a8830a1484ba469e845c37bb0af32f044bc8fd0c892c8878419e8587",
            "206c376f9da440494e18b283803698ed13249af93be3e99f58f42d7d82744d3d15ac",
        ),
        // Empty incorrect multi-signature test
        (
            "00004c875220ecdd9ec9f2c53ed8e5a170cc88354e133299022da55e1e8bd3c61d8b9dcbd7df2068f191b6aca3d9d8cfa2edb0c44a10fc87dc36b62e1d02228257ccdf979b1fce20b1503ef14aa6773ba3a1f012dbea2992e181766c35c5bc17465b5f57807540bf2006e161ced6b77c11b9a317080a899121a9c6df30a76490402f9a3b7e18bce97b54ae",
            "aa2071b6c2c604a8830a1484ba469e845c37bb0af32f044bc8fd0c892c8878419e8587",
            "206c376f9da440494e18b283803698ed13249af93be3e99f58f42d7d82744d3d15ac",
        ),
        // Non-push only script sig test
        (
            "5175",
            "51",
            "",
        ),
    ];

    println!("Regenerating test vectors with BLAKE3...\n");

    for (i, (sig_script, script_pub_key_1, script_pub_key_2)) in test_vectors.iter().enumerate() {
        println!("Test Vector #{}", i + 1);

        // Hash signature script
        let sig_script_bytes = hex_to_bytes(sig_script);
        let mut hasher = Hasher::new();
        hasher.update(&sig_script_bytes);
        let sig_script_hash = hasher.finalize();
        println!("New signature script hash: {}", sig_script_hash.to_hex());

        // Hash script pub key 1
        if !script_pub_key_1.is_empty() {
            let script_pub_key_1_bytes = hex_to_bytes(script_pub_key_1);
            let mut hasher = Hasher::new();
            hasher.update(&script_pub_key_1_bytes);
            let script_pub_key_1_hash = hasher.finalize();
            println!("New script pub key 1 hash: {}", script_pub_key_1_hash.to_hex());
        }

        // Hash script pub key 2
        if !script_pub_key_2.is_empty() {
            let script_pub_key_2_bytes = hex_to_bytes(script_pub_key_2);
            let mut hasher = Hasher::new();
            hasher.update(&script_pub_key_2_bytes);
            let script_pub_key_2_hash = hasher.finalize();
            println!("New script pub key 2 hash: {}", script_pub_key_2_hash.to_hex());
        }

        println!();
    }
}
