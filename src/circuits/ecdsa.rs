// Patterned after [halo2wrong ECDSA](https://github.com/privacy-scaling-explorations/halo2wrong/blob/master/ecdsa/src/ecdsa.rs)
use ecc::integer::{IntegerInstructions, Range};
use ecc::maingate::{MainGate, RangeChip, RangeInstructions, RegionCtx};
use ecc::GeneralEccChip;
use ecdsa::ecdsa::{AssignedEcdsaSig, AssignedPublicKey, EcdsaChip, EcdsaConfig};
use halo2_proofs::halo2curves::{
    bn256::Fr as Fp, secp256k1::Secp256k1Affine as Secp256k1, CurveAffine,
};
use halo2_proofs::{circuit::*, plonk::*};

const BIT_LEN_LIMB: usize = 68;
const NUMBER_OF_LIMBS: usize = 4;

#[derive(Default, Clone)]
pub struct EcdsaVerifyCircuit {
    pub public_key: Value<Secp256k1>,
    pub signature: Value<(
        <Secp256k1 as CurveAffine>::ScalarExt,
        <Secp256k1 as CurveAffine>::ScalarExt,
    )>,
    pub msg_hash: Value<<Secp256k1 as CurveAffine>::ScalarExt>,

    pub aux_generator: Secp256k1,
    pub window_size: usize,
}

impl Circuit<Fp> for EcdsaVerifyCircuit {
    type Config = EcdsaConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<Fp>) -> Self::Config {
        let (rns_base, rns_scalar) =
            GeneralEccChip::<Secp256k1, Fp, NUMBER_OF_LIMBS, BIT_LEN_LIMB>::rns();
        let main_gate_config = MainGate::<Fp>::configure(meta);
        let mut overflow_bit_lens: Vec<usize> = vec![];
        overflow_bit_lens.extend(rns_base.overflow_lengths());
        overflow_bit_lens.extend(rns_scalar.overflow_lengths());
        let composition_bit_lens = vec![BIT_LEN_LIMB / NUMBER_OF_LIMBS];

        let range_config = RangeChip::<Fp>::configure(
            meta,
            &main_gate_config,
            composition_bit_lens,
            overflow_bit_lens,
        );

        EcdsaConfig::new(range_config, main_gate_config)
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<Fp>,
    ) -> Result<(), Error> {
        let mut ecc_chip = GeneralEccChip::<Secp256k1, Fp, NUMBER_OF_LIMBS, BIT_LEN_LIMB>::new(
            config.ecc_chip_config(),
        );

        layouter.assign_region(
            || "assign aux values",
            |region| {
                let offset = 0;
                let ctx = &mut RegionCtx::new(region, offset);

                ecc_chip.assign_aux_generator(ctx, Value::known(self.aux_generator))?;
                ecc_chip.assign_aux(ctx, self.window_size, 2)?;
                Ok(())
            },
        )?;

        let ecdsa_chip = EcdsaChip::new(ecc_chip.clone());
        let scalar_chip = ecc_chip.scalar_field_chip();

        layouter.assign_region(
            || "ecdsa verify region",
            |region| {
                let offset = 0;
                let ctx = &mut RegionCtx::new(region, offset);

                let r = self.signature.map(|signature| signature.0);
                let s = self.signature.map(|signature| signature.1);
                let integer_r = ecc_chip.new_unassigned_scalar(r);
                let integer_s = ecc_chip.new_unassigned_scalar(s);
                let msg_hash = ecc_chip.new_unassigned_scalar(self.msg_hash);

                let r_assigned = scalar_chip.assign_integer(ctx, integer_r, Range::Remainder)?;
                let s_assigned = scalar_chip.assign_integer(ctx, integer_s, Range::Remainder)?;
                let sig = AssignedEcdsaSig {
                    r: r_assigned,
                    s: s_assigned,
                };

                let pk_in_circuit = ecc_chip.assign_point(ctx, self.public_key)?;
                let pk_assigned = AssignedPublicKey {
                    point: pk_in_circuit,
                };
                let msg_hash = scalar_chip.assign_integer(ctx, msg_hash, Range::Remainder)?;
                ecdsa_chip.verify(ctx, &sig, &pk_assigned, &msg_hash)
            },
        )?;

        let range_chip = RangeChip::<Fp>::new(config.range_config);
        range_chip.load_table(&mut layouter)?;

        Ok(())
    }
}
