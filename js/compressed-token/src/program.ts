import {
    PublicKey,
    Keypair,
    TransactionInstruction,
    SystemProgram,
    Connection,
    AddressLookupTableProgram,
    AccountMeta,
} from '@solana/web3.js';
import { BN, Program, AnchorProvider, setProvider } from '@coral-xyz/anchor';
import { IDL, LightCompressedToken } from './idl/light_compressed_token';
import {
    CompressedProof,
    LightSystemProgram,
    ParsedTokenAccount,
    TokenTransferOutputData,
    bn,
    confirmConfig,
    CompressedTokenInstructionDataTransfer,
    defaultStaticAccountsStruct,
    sumUpLamports,
    toArray,
    useWallet,
    validateSameOwner,
    validateSufficientBalance,
    defaultTestStateTreeAccounts,
} from '@lightprotocol/stateless.js';
import {
    MINT_SIZE,
    TOKEN_PROGRAM_ID,
    createInitializeMint2Instruction,
    createMintToInstruction,
} from '@solana/spl-token';
import { CPI_AUTHORITY_SEED, POOL_SEED } from './constants';
import { packCompressedTokenAccounts } from './instructions/pack-compressed-token-accounts';

type CompressParams = {
    /**
     * The payer of the transaction.
     */
    payer: PublicKey;
    /**
     * owner of the *uncompressed* token account.
     */
    owner: PublicKey;
    /**
     * source (associated) token account address.
     */
    source: PublicKey;
    /**
     * owner of the compressed token account.
     * To compress to a batch of recipients, pass an array of PublicKeys.
     */
    toAddress: PublicKey | PublicKey[];
    /**
     * Mint address of the token to compress.
     */
    mint: PublicKey;
    /**
     * amount of tokens to compress.
     */
    amount: number | BN | number[] | BN[];
    /**
     * The state tree that the tx output should be inserted into. Defaults to a
     * public state tree if unspecified.
     */
    outputStateTree?: PublicKey;
};

type DecompressParams = {
    /**
     * The payer of the transaction.
     */
    payer: PublicKey;
    /**
     * input state to be consumed
     */
    inputCompressedTokenAccounts: ParsedTokenAccount[];
    /**
     * address of **uncompressed** destination token account.
     */
    toAddress: PublicKey;
    /**
     * amount of tokens to decompress.
     */
    amount: number | BN;
    /**
     * The recent state root indices of the input state. The expiry is tied to
     * the proof.
     */
    recentInputStateRootIndices: number[];
    /**
     * The recent validity proof for state inclusion of the input state. It
     * expires after n slots.
     */
    recentValidityProof: CompressedProof;
    /**
     * The state tree that the change tx output should be inserted into.
     * Defaults to a public state tree if unspecified.
     */
    outputStateTree?: PublicKey;
};

export type TransferParams = {
    /**
     * The payer of the transaction
     */
    payer: PublicKey;
    /**
     * The input state to be consumed
     */
    inputCompressedTokenAccounts: ParsedTokenAccount[];
    /**
     * Recipient address
     */
    toAddress: PublicKey;
    /**
     * Amount of tokens to transfer
     */
    amount: BN | number;
    /**
     * The recent state root indices of the input state. The expiry is tied to
     * the proof.

     */
    recentInputStateRootIndices: number[];
    /**
     * The recent validity proof for state inclusion of the input state. It
     * expires after n slots.
     */
    recentValidityProof: CompressedProof;
    /**
     * The state trees that the tx output should be inserted into. This can be a
     * single PublicKey or an array of PublicKey. Defaults to the 0th state tree
     * of input state.
     */
    outputStateTrees?: PublicKey[] | PublicKey;
};

/**
 * Create Mint account for compressed Tokens
 */
export type CreateMintParams = {
    /**
     * Tx feepayer
     */
    feePayer: PublicKey;
    /**
     * Mint authority
     */
    authority: PublicKey;
    /**
     * Mint public key
     */
    mint: PublicKey;
    /**
     * Mint decimals
     */
    decimals: number;
    /**
     * Optional: freeze authority
     */
    freezeAuthority: PublicKey | null;
    /**
     * lamport amount for mint account rent exemption
     */
    rentExemptBalance: number;
};

/**
 * Parameters for merging compressed token accounts
 */
export type MergeTokenAccountsParams = {
    /**
     * Tx feepayer
     */
    payer: PublicKey;
    /**
     * Owner of the token accounts to be merged
     */
    owner: PublicKey;
    /**
     * Mint public key
     */
    mint: PublicKey;
    /**
     * Array of compressed token accounts to merge
     */
    inputCompressedTokenAccounts: ParsedTokenAccount[];
    /**
     * Optional: Public key of the state tree to merge into
     */
    outputStateTree: PublicKey;
    /**
     * Optional: Recent validity proof for state inclusion
     */
    recentValidityProof: CompressedProof;
    /**
     * Optional: Recent state root indices of the input state
     */
    recentInputStateRootIndices: number[];
};

/**
 * Create compressed token accounts
 */
export type MintToParams = {
    /**
     * Tx feepayer
     */
    feePayer: PublicKey;
    /**
     * Mint authority
     */
    authority: PublicKey;
    /**
     * Mint public key
     */
    mint: PublicKey;
    /**
     * The Solana Public Keys to mint to.
     */
    toPubkey: PublicKey[] | PublicKey;
    /**
     * The amount of compressed tokens to mint.
     */
    amount: BN | BN[] | number | number[];
    /**
     * Public key of the state tree to mint into. Defaults to a public state
     * tree if unspecified.
     */
    merkleTree?: PublicKey;
};

/**
 * Register an existing SPL mint account to the compressed token program
 * Creates an omnibus account for the mint
 */
export type RegisterMintParams = {
    /** Tx feepayer */
    feePayer: PublicKey;
    /** Mint public key */
    mint: PublicKey;
};

/**
 * Mint from existing SPL mint to compressed token accounts
 */
export type ApproveAndMintToParams = {
    /**
     * Tx feepayer
     */
    feePayer: PublicKey;
    /**
     * Mint authority
     */
    authority: PublicKey;
    /**
     * Mint authority (associated) token account
     */
    authorityTokenAccount: PublicKey;
    /**
     * Mint public key
     */
    mint: PublicKey;
    /**
     * The Solana Public Key to mint to.
     */
    toPubkey: PublicKey;
    /**
     * The amount of compressed tokens to mint.
     */
    amount: BN | number;
    /**
     * Public key of the state tree to mint into. Defaults to a public state
     * tree if unspecified.
     */
    merkleTree?: PublicKey;
};

export type CreateTokenProgramLookupTableParams = {
    /**
     * The payer of the transaction.
     */
    payer: PublicKey;
    /**
     * The authority of the transaction.
     */
    authority: PublicKey;
    /**
     *  Recently finalized Solana slot.
     */
    recentSlot: number;
    /**
     * Optional Mint addresses to store in the lookup table.
     */
    mints?: PublicKey[];
    /**
     * Optional additional addresses to store in the lookup table.
     */
    remainingAccounts?: PublicKey[];
};

/**
 * Sum up the token amounts of the compressed token accounts
 */
export const sumUpTokenAmount = (accounts: ParsedTokenAccount[]): BN => {
    return accounts.reduce(
        (acc, account: ParsedTokenAccount) => acc.add(account.parsed.amount),
        bn(0),
    );
};

/**
 * Validate that all the compressed token accounts are owned by the same owner.
 */
export const validateSameTokenOwner = (accounts: ParsedTokenAccount[]) => {
    const owner = accounts[0].parsed.owner;
    accounts.forEach(acc => {
        if (!acc.parsed.owner.equals(owner)) {
            throw new Error('Token accounts must be owned by the same owner');
        }
    });
};

/**
 * Parse compressed token accounts to get the mint, current owner and delegate.
 */
export const parseTokenData = (
    compressedTokenAccounts: ParsedTokenAccount[],
) => {
    const mint = compressedTokenAccounts[0].parsed.mint;
    const currentOwner = compressedTokenAccounts[0].parsed.owner;
    const delegate = compressedTokenAccounts[0].parsed.delegate;

    return { mint, currentOwner, delegate };
};

/**
 * Create the output state for a transfer transaction.
 * @param inputCompressedTokenAccounts  Input state
 * @param toAddress                     Recipient address
 * @param amount                        Amount of tokens to transfer
 * @returns                             Output token data for the transfer
 *                                      instruction
 */
export function createTransferOutputState(
    inputCompressedTokenAccounts: ParsedTokenAccount[],
    toAddress: PublicKey,
    amount: number | BN,
): TokenTransferOutputData[] {
    amount = bn(amount);
    const inputAmount = sumUpTokenAmount(inputCompressedTokenAccounts);
    const inputLamports = sumUpLamports(
        inputCompressedTokenAccounts.map(acc => acc.compressedAccount),
    );

    const changeAmount = inputAmount.sub(amount);

    validateSufficientBalance(changeAmount);

    if (changeAmount.eq(bn(0)) && inputLamports.eq(bn(0))) {
        return [
            {
                owner: toAddress,
                amount,
                lamports: inputLamports,
                tlv: null,
            },
        ];
    }

    /// validates token program
    validateSameOwner(
        inputCompressedTokenAccounts.map(acc => acc.compressedAccount),
    );
    validateSameTokenOwner(inputCompressedTokenAccounts);

    const outputCompressedAccounts: TokenTransferOutputData[] = [
        {
            owner: inputCompressedTokenAccounts[0].parsed.owner,
            amount: changeAmount,
            lamports: inputLamports,
            tlv: null,
        },
        {
            owner: toAddress,
            amount,
            lamports: bn(0),
            tlv: null,
        },
    ];
    return outputCompressedAccounts;
}

/**
 * Create the output state for a compress transaction.
 * @param inputCompressedTokenAccounts  Input state
 * @param amount                        Amount of tokens to compress
 * @returns                             Output token data for the compress
 *                                      instruction
 */
export function createDecompressOutputState(
    inputCompressedTokenAccounts: ParsedTokenAccount[],
    amount: number | BN,
): TokenTransferOutputData[] {
    amount = bn(amount);
    const inputLamports = sumUpLamports(
        inputCompressedTokenAccounts.map(acc => acc.compressedAccount),
    );
    const inputAmount = sumUpTokenAmount(inputCompressedTokenAccounts);
    const changeAmount = inputAmount.sub(amount);

    validateSufficientBalance(changeAmount);

    /// lamports gets decompressed
    if (changeAmount.eq(bn(0)) && inputLamports.eq(bn(0))) {
        return [];
    }

    validateSameOwner(
        inputCompressedTokenAccounts.map(acc => acc.compressedAccount),
    );
    validateSameTokenOwner(inputCompressedTokenAccounts);

    const tokenTransferOutputs: TokenTransferOutputData[] = [
        {
            owner: inputCompressedTokenAccounts[0].parsed.owner,
            amount: changeAmount,
            lamports: inputLamports,
            tlv: null,
        },
    ];
    return tokenTransferOutputs;
}

export type CompressSplTokenAccountParams = {
    /**
     * Tx feepayer
     */
    feePayer: PublicKey;
    /**
     * Authority that owns the token account
     */
    authority: PublicKey;
    /**
     * Token account to compress
     */
    tokenAccount: PublicKey;
    /**
     * Mint public key
     */
    mint: PublicKey;
    /**
     * Optional: remaining amount to leave in token account. Default: 0
     */
    remainingAmount?: BN;
    /**
     * The state tree that the compressed token account should be inserted into.
     */
    outputStateTree: PublicKey;
};

export class CompressedTokenProgram {
    /**
     * @internal
     */
    constructor() {}

    /**
     * Public key that identifies the CompressedPda program
     */
    static programId: PublicKey = new PublicKey(
        'cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m',
    );

    private static _program: Program<LightCompressedToken> | null = null;

    /** @internal */
    static get program(): Program<LightCompressedToken> {
        if (!this._program) {
            this.initializeProgram();
        }
        return this._program!;
    }

    /**
     * @internal
     * Initializes the program statically if not already initialized.
     */
    private static initializeProgram() {
        if (!this._program) {
            /// Note: We can use a mock connection because we're using the
            /// program only for serde and building instructions, not for
            /// interacting with the network.
            const mockKeypair = Keypair.generate();
            const mockConnection = new Connection(
                'http://127.0.0.1:8899',
                'confirmed',
            );
            const mockProvider = new AnchorProvider(
                mockConnection,
                useWallet(mockKeypair),
                confirmConfig,
            );
            setProvider(mockProvider);
            this._program = new Program(IDL, this.programId, mockProvider);
        }
    }

    /** @internal */
    static deriveTokenPoolPda(mint: PublicKey): PublicKey {
        const seeds = [POOL_SEED, mint.toBuffer()];
        const [address, _] = PublicKey.findProgramAddressSync(
            seeds,
            this.programId,
        );
        return address;
    }

    /** @internal */
    static get deriveCpiAuthorityPda(): PublicKey {
        const [address, _] = PublicKey.findProgramAddressSync(
            [CPI_AUTHORITY_SEED],
            this.programId,
        );
        return address;
    }

    /**
     * Construct createMint instruction for compressed tokens
     */
    static async createMint(
        params: CreateMintParams,
    ): Promise<TransactionInstruction[]> {
        const { mint, authority, feePayer, rentExemptBalance } = params;

        /// Create and initialize SPL Mint account
        const createMintAccountInstruction = SystemProgram.createAccount({
            fromPubkey: feePayer,
            lamports: rentExemptBalance,
            newAccountPubkey: mint,
            programId: TOKEN_PROGRAM_ID,
            space: MINT_SIZE,
        });

        const initializeMintInstruction = createInitializeMint2Instruction(
            mint,
            params.decimals,
            authority,
            params.freezeAuthority,
            TOKEN_PROGRAM_ID,
        );

        const ix = await this.createTokenPool({
            feePayer,
            mint,
        });

        return [createMintAccountInstruction, initializeMintInstruction, ix];
    }

    /**
     * Enable compression for an existing SPL mint, creating an omnibus account.
     * For new mints, use `CompressedTokenProgram.createMint`.
     */
    static async createTokenPool(
        params: RegisterMintParams,
    ): Promise<TransactionInstruction> {
        const { mint, feePayer } = params;

        const tokenPoolPda = this.deriveTokenPoolPda(mint);

        const ix = await this.program.methods
            .createTokenPool()
            .accounts({
                mint,
                feePayer,
                tokenPoolPda,
                systemProgram: SystemProgram.programId,
                tokenProgram: TOKEN_PROGRAM_ID,
                cpiAuthorityPda: this.deriveCpiAuthorityPda,
            })
            .instruction();

        return ix;
    }

    /**
     * Construct mintTo instruction for compressed tokens
     */
    static async mintTo(params: MintToParams): Promise<TransactionInstruction> {
        const systemKeys = defaultStaticAccountsStruct();

        const { mint, feePayer, authority, merkleTree, toPubkey, amount } =
            params;

        const tokenPoolPda = this.deriveTokenPoolPda(mint);

        const amounts = toArray<BN | number>(amount).map(amount => bn(amount));

        const toPubkeys = toArray(toPubkey);

        if (amounts.length !== toPubkeys.length) {
            throw new Error(
                'Amount and toPubkey arrays must have the same length',
            );
        }

        const instruction = await this.program.methods
            .mintTo(toPubkeys, amounts, null)
            .accounts({
                feePayer,
                authority,
                cpiAuthorityPda: this.deriveCpiAuthorityPda,
                mint,
                tokenPoolPda,
                tokenProgram: TOKEN_PROGRAM_ID,
                lightSystemProgram: LightSystemProgram.programId,
                registeredProgramPda: systemKeys.registeredProgramPda,
                noopProgram: systemKeys.noopProgram,
                accountCompressionAuthority:
                    systemKeys.accountCompressionAuthority,
                accountCompressionProgram: systemKeys.accountCompressionProgram,
                merkleTree:
                    merkleTree ?? defaultTestStateTreeAccounts().merkleTree,
                selfProgram: this.programId,
                solPoolPda: null,
            })
            .instruction();
        return instruction;
    }

    /// TODO: add compressBatch functionality for batch minting
    /**
     * Mint tokens from registed SPL mint account to a compressed account
     */
    static async approveAndMintTo(params: ApproveAndMintToParams) {
        const {
            mint,
            feePayer,
            authorityTokenAccount,
            authority,
            merkleTree,
            toPubkey,
        } = params;

        const amount: bigint = BigInt(params.amount.toString());

        /// 1. Mint to existing ATA of mintAuthority.
        const splMintToInstruction = createMintToInstruction(
            mint,
            authorityTokenAccount,
            authority,
            amount,
        );

        /// 2. Compress from mint authority ATA to recipient compressed account
        const compressInstruction = await this.compress({
            payer: feePayer,
            owner: authority,
            source: authorityTokenAccount,
            toAddress: toPubkey,
            mint,
            amount: params.amount,
            outputStateTree: merkleTree,
        });

        return [splMintToInstruction, compressInstruction];
    }
    /**
     * Construct transfer instruction for compressed tokens
     */
    static async transfer(
        params: TransferParams,
    ): Promise<TransactionInstruction> {
        const {
            payer,
            inputCompressedTokenAccounts,
            recentInputStateRootIndices,
            recentValidityProof,
            amount,
            outputStateTrees,
            toAddress,
        } = params;

        const tokenTransferOutputs: TokenTransferOutputData[] =
            createTransferOutputState(
                inputCompressedTokenAccounts,
                toAddress,
                amount,
            );
        const {
            inputTokenDataWithContext,
            packedOutputTokenData,
            remainingAccountMetas,
        } = packCompressedTokenAccounts({
            inputCompressedTokenAccounts,
            outputStateTrees,
            rootIndices: recentInputStateRootIndices,
            tokenTransferOutputs,
        });

        const { mint, currentOwner } = parseTokenData(
            inputCompressedTokenAccounts,
        );

        const data: CompressedTokenInstructionDataTransfer = {
            proof: recentValidityProof,
            mint,
            delegatedTransfer: null, // TODO: implement
            inputTokenDataWithContext,
            outputCompressedAccounts: packedOutputTokenData,
            compressOrDecompressAmount: null,
            isCompress: false,
            cpiContext: null,
            lamportsChangeAccountMerkleTreeIndex: null,
        };

        const encodedData = this.program.coder.types.encode(
            'CompressedTokenInstructionDataTransfer',
            data,
        );

        const {
            accountCompressionAuthority,
            noopProgram,
            registeredProgramPda,
            accountCompressionProgram,
        } = defaultStaticAccountsStruct();

        const instruction = await this.program.methods
            .transfer(encodedData)
            .accounts({
                feePayer: payer!,
                authority: currentOwner!,
                cpiAuthorityPda: this.deriveCpiAuthorityPda,
                lightSystemProgram: LightSystemProgram.programId,
                registeredProgramPda: registeredProgramPda,
                noopProgram: noopProgram,
                accountCompressionAuthority: accountCompressionAuthority,
                accountCompressionProgram: accountCompressionProgram,
                selfProgram: this.programId,
                tokenPoolPda: null,
                compressOrDecompressTokenAccount: null,
                tokenProgram: null,
            })
            .remainingAccounts(remainingAccountMetas)
            .instruction();

        return instruction;
    }

    /**
     * Create lookup table instructions for the token program's default accounts.
     */
    static async createTokenProgramLookupTable(
        params: CreateTokenProgramLookupTableParams,
    ) {
        const { authority, mints, recentSlot, payer, remainingAccounts } =
            params;

        const [createInstruction, lookupTableAddress] =
            AddressLookupTableProgram.createLookupTable({
                authority,
                payer: authority,
                recentSlot,
            });

        let optionalMintKeys: PublicKey[] = [];
        if (mints) {
            optionalMintKeys = [
                ...mints,
                ...mints.map(mint => this.deriveTokenPoolPda(mint)),
            ];
        }

        const extendInstruction = AddressLookupTableProgram.extendLookupTable({
            payer,
            authority,
            lookupTable: lookupTableAddress,
            addresses: [
                this.deriveCpiAuthorityPda,
                LightSystemProgram.programId,
                defaultStaticAccountsStruct().registeredProgramPda,
                defaultStaticAccountsStruct().noopProgram,
                defaultStaticAccountsStruct().accountCompressionAuthority,
                defaultStaticAccountsStruct().accountCompressionProgram,
                defaultTestStateTreeAccounts().merkleTree,
                defaultTestStateTreeAccounts().nullifierQueue,
                defaultTestStateTreeAccounts().addressTree,
                defaultTestStateTreeAccounts().addressQueue,
                this.programId,
                TOKEN_PROGRAM_ID,
                authority,
                ...optionalMintKeys,
                ...(remainingAccounts ?? []),
            ],
        });

        return {
            instructions: [createInstruction, extendInstruction],
            address: lookupTableAddress,
        };
    }

    /**
     * Create compress instruction
     * @returns compressInstruction
     */
    static async compress(
        params: CompressParams,
    ): Promise<TransactionInstruction> {
        const { payer, owner, source, toAddress, mint, outputStateTree } =
            params;

        if (Array.isArray(params.amount) !== Array.isArray(params.toAddress)) {
            throw new Error(
                'Both amount and toAddress must be arrays or both must be single values',
            );
        }

        let tokenTransferOutputs: TokenTransferOutputData[];

        if (Array.isArray(params.amount) && Array.isArray(params.toAddress)) {
            if (params.amount.length !== params.toAddress.length) {
                throw new Error(
                    'Amount and toAddress arrays must have the same length',
                );
            }
            tokenTransferOutputs = params.amount.map((amt, index) => {
                const amount = bn(amt);
                return {
                    owner: (params.toAddress as PublicKey[])[index],
                    amount,
                    lamports: bn(0),
                    tlv: null,
                };
            });
        } else {
            tokenTransferOutputs = [
                {
                    owner: toAddress as PublicKey,
                    amount: bn(params.amount as number | BN),
                    lamports: bn(0),
                    tlv: null,
                },
            ];
        }

        const {
            inputTokenDataWithContext,
            packedOutputTokenData,
            remainingAccountMetas,
        } = packCompressedTokenAccounts({
            inputCompressedTokenAccounts: [],
            outputStateTrees: outputStateTree,
            rootIndices: [],
            tokenTransferOutputs,
        });

        const data: CompressedTokenInstructionDataTransfer = {
            proof: null,
            mint,
            delegatedTransfer: null, // TODO: implement
            inputTokenDataWithContext,
            outputCompressedAccounts: packedOutputTokenData,
            compressOrDecompressAmount: Array.isArray(params.amount)
                ? params.amount
                      .map(amt => new BN(amt))
                      .reduce((sum, amt) => sum.add(amt), new BN(0))
                : new BN(params.amount),
            isCompress: true,
            cpiContext: null,
            lamportsChangeAccountMerkleTreeIndex: null,
        };

        const encodedData = this.program.coder.types.encode(
            'CompressedTokenInstructionDataTransfer',
            data,
        );

        const instruction = await this.program.methods
            .transfer(encodedData)
            .accounts({
                feePayer: payer,
                authority: owner,
                cpiAuthorityPda: this.deriveCpiAuthorityPda,
                lightSystemProgram: LightSystemProgram.programId,
                registeredProgramPda:
                    defaultStaticAccountsStruct().registeredProgramPda,
                noopProgram: defaultStaticAccountsStruct().noopProgram,
                accountCompressionAuthority:
                    defaultStaticAccountsStruct().accountCompressionAuthority,
                accountCompressionProgram:
                    defaultStaticAccountsStruct().accountCompressionProgram,
                selfProgram: this.programId,
                tokenPoolPda: this.deriveTokenPoolPda(mint),
                compressOrDecompressTokenAccount: source, // token
                tokenProgram: TOKEN_PROGRAM_ID,
            })
            .remainingAccounts(remainingAccountMetas)
            .instruction();

        return instruction;
    }

    /**
     * Construct decompress instruction
     */
    static async decompress(
        params: DecompressParams,
    ): Promise<TransactionInstruction> {
        const {
            payer,
            inputCompressedTokenAccounts,
            toAddress,
            outputStateTree,
            recentValidityProof,
            recentInputStateRootIndices,
        } = params;
        const amount = bn(params.amount);

        const tokenTransferOutputs = createDecompressOutputState(
            inputCompressedTokenAccounts,
            amount,
        );

        /// Pack
        const {
            inputTokenDataWithContext,
            packedOutputTokenData,
            remainingAccountMetas,
        } = packCompressedTokenAccounts({
            inputCompressedTokenAccounts,
            outputStateTrees: outputStateTree,
            rootIndices: recentInputStateRootIndices,
            tokenTransferOutputs: tokenTransferOutputs,
        });

        const { mint, currentOwner } = parseTokenData(
            inputCompressedTokenAccounts,
        );

        const data: CompressedTokenInstructionDataTransfer = {
            proof: recentValidityProof,
            mint,
            delegatedTransfer: null, // TODO: implement
            inputTokenDataWithContext,
            outputCompressedAccounts: packedOutputTokenData,
            compressOrDecompressAmount: amount,
            isCompress: false,
            cpiContext: null,
            lamportsChangeAccountMerkleTreeIndex: null,
        };

        const encodedData = this.program.coder.types.encode(
            'CompressedTokenInstructionDataTransfer',
            data,
        );

        const {
            accountCompressionAuthority,
            noopProgram,
            registeredProgramPda,
            accountCompressionProgram,
        } = defaultStaticAccountsStruct();

        const instruction = await this.program.methods
            .transfer(encodedData)
            .accounts({
                feePayer: payer,
                authority: currentOwner,
                cpiAuthorityPda: this.deriveCpiAuthorityPda,
                lightSystemProgram: LightSystemProgram.programId,
                registeredProgramPda: registeredProgramPda,
                noopProgram: noopProgram,
                accountCompressionAuthority: accountCompressionAuthority,
                accountCompressionProgram: accountCompressionProgram,
                selfProgram: this.programId,
                tokenPoolPda: this.deriveTokenPoolPda(mint),
                compressOrDecompressTokenAccount: toAddress,
                tokenProgram: TOKEN_PROGRAM_ID,
            })
            .remainingAccounts(remainingAccountMetas)
            .instruction();

        return instruction;
    }

    static async mergeTokenAccounts(
        params: MergeTokenAccountsParams,
    ): Promise<TransactionInstruction[]> {
        const {
            payer,
            owner,
            inputCompressedTokenAccounts,
            outputStateTree,
            recentValidityProof,
            recentInputStateRootIndices,
        } = params;

        if (inputCompressedTokenAccounts.length > 3) {
            throw new Error('Cannot merge more than 3 token accounts at once');
        }

        const ix = await this.transfer({
            payer,
            inputCompressedTokenAccounts,
            toAddress: owner,
            amount: inputCompressedTokenAccounts.reduce(
                (sum, account) => sum.add(account.parsed.amount),
                new BN(0),
            ),
            outputStateTrees: outputStateTree,
            recentInputStateRootIndices,
            recentValidityProof,
        });

        return [ix];
    }

    static async compressSplTokenAccount(
        params: CompressSplTokenAccountParams,
    ): Promise<TransactionInstruction> {
        const {
            feePayer,
            authority,
            tokenAccount,
            mint,
            remainingAmount,
            outputStateTree,
        } = params;

        const remainingAccountMetas: AccountMeta[] = [
            {
                pubkey: outputStateTree,
                isSigner: false,
                isWritable: true,
            },
        ];

        const instruction = await this.program.methods
            .compressSplTokenAccount(authority, remainingAmount ?? null, null)
            .accounts({
                feePayer,
                authority,
                cpiAuthorityPda: this.deriveCpiAuthorityPda,
                lightSystemProgram: LightSystemProgram.programId,
                registeredProgramPda:
                    defaultStaticAccountsStruct().registeredProgramPda,
                noopProgram: defaultStaticAccountsStruct().noopProgram,
                accountCompressionAuthority:
                    defaultStaticAccountsStruct().accountCompressionAuthority,
                accountCompressionProgram:
                    defaultStaticAccountsStruct().accountCompressionProgram,
                selfProgram: this.programId,
                tokenPoolPda: this.deriveTokenPoolPda(mint),
                compressOrDecompressTokenAccount: tokenAccount,
                tokenProgram: TOKEN_PROGRAM_ID,
                systemProgram: SystemProgram.programId,
            })
            .remainingAccounts(remainingAccountMetas)
            .instruction();

        return instruction;
    }
}
